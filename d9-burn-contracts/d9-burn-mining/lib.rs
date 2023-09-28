#![cfg_attr(not(feature = "std"), no_std, no_main)]

use d9_burn_common::{ Account, Error, D9Environment };

#[ink::contract(env = D9Environment)]
mod d9_burn_mining {
    use super::*;
    use ink::storage::Mapping;
    use sp_arithmetic::{ Rounding::NearestPrefDown, Percent };

    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        total_amount_burned: Balance,
        /// the controller of this contract
        master_controller_contract: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
    }

    impl D9burnMining {
        #[ink(constructor)]
        pub fn new(master_controller_contract: AccountId) -> Self {
            Self {
                total_amount_burned: Default::default(),
                master_controller_contract,
                accounts: Default::default(),
            }
        }

        /// Executes a restricted burn operation.
        ///
        /// This function allows only the master controller contract to initiate a burn operation for a given account.
        /// It ensures the calling contract has the correct permission to burn tokens, then performs the burn operation
        /// and returns the net balance due (outstanding balance minus paid balance) for the account.
        ///
        /// # Parameters
        /// - `account_id`: The account ID for which the burn operation should be executed.
        /// - `burn_amount`: The amount of tokens to be burned from the specified account.
        ///
        /// # Returns
        /// - `Ok(Balance)`: If the burn operation is successful, it returns the net balance due for the account.
        /// - `Err(Error::RestrictedFunction)`: If the calling contract is not the master controller contract.
        /// - `Err(Error)`: Other errors as reported by the internal `_burn` function.
        ///
        /// # Note
        /// The function performs permission checks to ensure only authorized contracts can initiate the burn.
        #[ink(message)]
        pub fn controller_restricted_burn(
            &mut self,
            account_id: AccountId,
            burn_amount: Balance
        ) -> Result<Balance, Error> {
            if self.env().caller() != self.master_controller_contract {
                return Err(Error::RestrictedFunction);
            }
            let burn_result = self._burn(account_id, burn_amount);
            if burn_result.is_err() {
                return Err(burn_result.unwrap_err());
            }
            let account = burn_result.unwrap();
            Ok(account.balance_due.saturating_sub(account.balance_paid))
        }

        /// Burns the specified amount from the given account, updating the total burned amount
        /// and the account's details.
        ///
        /// The function first checks if the specified burn amount is above a set threshold.
        /// If it's below this threshold, the burn request is rejected with a `BurnAmountBelowThreshold` error.
        ///
        /// The function transfers the burn amount to the `master_controller_contract`, updates the
        /// total burned amount of the contract, and then adjusts the details of the account in
        /// the following ways:
        ///
        /// 1. If the account already exists, the function updates the amount burned, the last burn timestamp,
        ///    and increments the balance due by three times the burned amount.
        /// 2. If the account doesn't exist, a new account is created with the provided burn details.
        ///
        /// An event `BurnExecuted` is emitted after a successful burn operation.
        ///
        /// # Parameters
        ///
        /// * `account_id`: The ID of the account from which the amount will be burned.
        /// * `amount`: The amount to be burned.
        ///
        /// # Returns
        ///
        /// A `Result` indicating the success or failure of the burn operation. On success,
        /// it returns the updated `Account` state, and on failure, it provides an associated `Error`.
        ///
        fn _burn(&mut self, account_id: AccountId, amount: Balance) -> Result<Account, Error> {
            if amount < 100_000_000 {
                return Err(Error::BurnAmountInsufficient);
            }
            self.env().transfer(self.master_controller_contract, amount).expect("Transfer failed.");
            self.total_amount_burned = self.total_amount_burned.saturating_add(amount);
            let maybe_account: Option<Account> = self.accounts.get(&account_id);
            let account = match maybe_account {
                Some(mut account) => {
                    account.amount_burned = account.amount_burned.saturating_add(amount);
                    account.last_burn = self.env().block_timestamp();
                    account.balance_due = account.balance_due.saturating_add(
                        amount.saturating_mul(3)
                    );
                    self.accounts.insert(self.env().caller(), &account);
                    account
                }
                None => {
                    let account = Account {
                        amount_burned: amount,
                        balance_due: amount.saturating_mul(3),
                        balance_paid: 0,
                        last_withdrawal: 0,
                        last_burn: self.env().block_timestamp(),
                    };
                    self.accounts.insert(self.env().caller(), &account);
                    account
                }
            };
            Ok(account)
        }

        /// Executes a withdrawal operation for a portfolio, restricted to the master controller contract.
        ///
        /// This function can only be invoked by the master controller contract and is used to process
        /// withdrawals for a given account from its portfolio. It calculates the allowed withdrawal amount,
        /// updates the account's records, and then returns the withdrawal amount along with the timestamp
        /// of the last withdrawal operation.
        ///
        /// # Parameters
        /// - `account_id`: The account ID for which the withdrawal should be executed.
        ///
        /// # Returns
        /// - `Ok((Balance, Timestamp))`: On successful withdrawal, returns the allowed withdrawal amount and
        ///   the timestamp of the last withdrawal.
        /// - `Err(Error::RestrictedFunction)`: If the calling contract is not the master controller contract.
        /// - `Err(Error::NoAccountFound)`: If the specified account does not exist.
        /// - `Err(Error)`: Other errors as reported by the internal `_update_account` function.
        ///
        /// # Note
        /// Permission checks are performed to ensure only the master controller contract can initiate the withdrawal.

        #[ink(message)]
        pub fn portfolio_executed_withdrawal(
            &mut self,
            account_id: AccountId
        ) -> Result<(Balance, Timestamp), Error> {
            if self.env().caller() != self.master_controller_contract {
                return Err(Error::RestrictedFunction);
            }
            let maybe_account: Option<Account> = self.accounts.get(&account_id);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let account = maybe_account.unwrap();
            let withdrawal_allowance = self._calculate_withdrawal(&account);
            let updated_account = self._update_account(
                account_id,
                account,
                withdrawal_allowance.clone()
            );
            if updated_account.is_err() {
                return Err(updated_account.unwrap_err());
            }

            Ok((withdrawal_allowance, updated_account.unwrap().last_withdrawal))
        }

        /// Calculates the allowed withdrawal amount for a given account based on the time since its last withdrawal.
        ///
        /// This function computes the amount an account can withdraw based on the days elapsed
        /// since its last withdrawal operation. The withdrawal amount is determined using a
        /// daily return percentage and is limited to the outstanding balance due to the account.
        ///
        /// # Parameters
        /// - `account`: A reference to the account structure for which the withdrawal allowance is to be calculated.
        ///
        /// # Returns
        /// - `Balance`: The calculated withdrawal amount for the given account.
        ///
        /// # Note
        /// The function ensures that withdrawal is only allowed once every 24 hours by checking the difference
        /// between the current block timestamp and the account's last withdrawal timestamp.
        /// The daily return percentage is fetched using the `_get_return_percent` internal function.
        fn _calculate_withdrawal(&self, account: &Account) -> Balance {
            pub const DAY: Timestamp = 86_400;
            let days_since_last_withdraw = self
                .env()
                .block_timestamp()
                .saturating_sub(account.last_withdrawal)
                .saturating_div(DAY);
            if days_since_last_withdraw == 0 {
                return 0;
            }
            let daily_return_percent = self._get_return_percent();
            let withdraw_allowance: Balance = {
                let allowance = daily_return_percent
                    .mul_floor(account.balance_due)
                    .saturating_mul(days_since_last_withdraw as u128);

                if allowance > account.balance_due {
                    account.balance_due
                } else {
                    allowance
                }
            };
            withdraw_allowance
        }

        /// Updates an account's records after a withdrawal operation.
        ///
        /// This function modifies an account's attributes, including its last withdrawal timestamp,
        /// total balance paid, and balance due. It then stores the updated account data.
        ///
        /// # Parameters
        /// - `account_id`: The unique identifier of the account being updated.
        /// - `account`: The current state of the account before the withdrawal.
        /// - `withdraw_allowance`: The allowed withdrawal amount for the account.
        ///
        /// # Returns
        /// - `Result<Account, Error>`: The updated account state if successful, or an error if the operation fails.
        ///
        /// # Note
        /// The function increments the `balance_paid` by the `withdraw_allowance` and sets the `last_withdrawal`
        /// timestamp to the current block timestamp. It also reduces the `balance_due` by the withdrawal amount.
        /// The modified account data is then stored back into the `accounts` storage map.
        fn _update_account(
            &mut self,
            account_id: AccountId,
            mut account: Account,
            withdraw_allowance: Balance
        ) -> Result<Account, Error> {
            let last_withdrawal = self.env().block_timestamp();
            account.balance_paid = account.balance_paid.saturating_add(withdraw_allowance);
            account.last_withdrawal = last_withdrawal;
            account.balance_due = account.balance_due.saturating_sub(withdraw_allowance);
            self.accounts.insert(account_id, &account);
            Ok(account.clone())
        }

        /// the returned percent is used for an accounts return based on the amount burned
        ///
        /// This function calculates the return percentage based on the total amount burned
        /// within the contract. The return percentage starts at 0.8% and is reduced by half
        /// for every 100_000_000_000_000 units over the first threshold of 200_000_000_000_000.
        ///
        /// # Parameters:
        ///
        /// - `&self`: A reference to the instance of the ink! contract.
        ///
        /// # Returns:
        ///
        /// Returns a `Percent` value representing the return percentage.
        ///
        fn _get_return_percent(&self) -> Percent {
            let first_threshold_amount: Balance = 200_000_000_000_000;
            // let mut percentage: f64 = 0.008;
            let percentage: Percent = Percent::from_rational(8u128, 1000u128);
            if self.total_amount_burned <= first_threshold_amount {
                return percentage;
            }

            let excess_amount: u128 =
                self.total_amount_burned.saturating_sub(first_threshold_amount);
            let reductions: u128 = excess_amount
                .saturating_div(100_000_000_000_000)
                .saturating_add(1);

            for _ in 0..reductions {
                percentage.saturating_div(Percent::from_rational(2u128, 1u128), NearestPrefDown);
            }
            // Percent::from_float(percentage)
            percentage
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;

        /// We test if the default constructor does its job.
        #[ink::test]
        fn default_works() {
            let d9_burn_mining = D9burnMining::default();
            assert_eq!(d9_burn_mining.get(), false);
        }

        //   /// We test a simple use case of our contract.
        //   #[ink::test]
        //   fn it_works() {
        //       let mut d9referral = D9BurnMining::new(false);
        //       assert_eq!(d9referral.get(), false);
        //       d9referral.flip();
        //       assert_eq!(d9referral.get(), true);
        //   }
    }
}
