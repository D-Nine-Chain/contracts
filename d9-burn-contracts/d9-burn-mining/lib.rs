#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract(env = D9Environment)]
mod d9_burn_mining {
    use d9_burn_common::{ Account, Error, D9Environment };
    use ink::storage::Mapping;
    use sp_arithmetic::Percent;
    //  use d9_chain_extension::D9Environment;

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        total_amount_burned: Balance,
        ///the contract that maintains account data for an account with respect to N different burn contracts
        master_portfolio_contract: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
    }

    impl D9burnMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(master_portfolio_contract: AccountId) -> Self {
            //todo define a struct to hold account data for what was burned and what was paid out
            Self {
                total_amount_burned: Default::default(),
                master_portfolio_contract,
                accounts: Default::default(),
            }
        }

        #[ink(message)]
        pub fn portfolio_executed_burn(
            &mut self,
            account_id: AccountId,
            burn_amount: Balance
        ) -> Result<Balance, Error> {
            if self.env().caller() != self.master_portfolio_contract {
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
        /// The function transfers the burn amount to the `master_portfolio_contract`, updates the
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
            self.env().transfer(self.master_portfolio_contract, amount).expect("Transfer failed.");
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

        #[ink(message)]
        pub fn portfolio_executed_withdrawal(
            &mut self,
            account_id: AccountId
        ) -> Result<(Balance, Timestamp), Error> {
            if self.env().caller() != self.master_portfolio_contract {
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
            let mut percentage: f64 = 0.008;

            if self.total_amount_burned <= first_threshold_amount {
                return Percent::from_float(percentage);
            }

            let excess_amount: u128 =
                self.total_amount_burned.saturating_sub(first_threshold_amount);
            let reductions: u128 = excess_amount
                .saturating_div(100_000_000_000_000)
                .saturating_add(1);

            for _ in 0..reductions {
                percentage /= 2.0;
            }
            Percent::from_float(percentage)
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
            let d9referral = D9BurnMining::default();
            assert_eq!(d9referral.get(), false);
        }

        /// We test a simple use case of our contract.
        #[ink::test]
        fn it_works() {
            let mut d9referral = D9BurnMining::new(false);
            assert_eq!(d9referral.get(), false);
            d9referral.flip();
            assert_eq!(d9referral.get(), true);
        }
    }
}
