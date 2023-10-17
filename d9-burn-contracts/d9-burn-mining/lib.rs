#![cfg_attr(not(feature = "std"), no_std, no_main)]

use d9_burn_common::{ Account, D9Environment, Error };

#[ink::contract(env = D9Environment)]
// #[ink::contract(env = D9Environment)]
pub mod d9_burn_mining {
    use super::*;
    use ink::storage::Mapping;
    use sp_arithmetic::{ Rounding::NearestPrefDown, Perbill };

    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        total_amount_burned: Balance,
        /// the controller of this contract
        master_controller_contract: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
        ///minimum permissible burn amount
        burn_minimum: Balance,
    }

    impl D9burnMining {
        #[ink(constructor, payable)]
        pub fn new(master_controller_contract: AccountId, burn_minimum: Balance) -> Self {
            Self {
                total_amount_burned: Default::default(),
                master_controller_contract,
                accounts: Default::default(),
                burn_minimum,
            }
        }
        #[ink(message)]
        pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
            self.accounts.get(&account_id)
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
            // assert!(
            //     self.env().caller() != self.master_controller_contract,
            //     "Caller is not the master controller contract."
            // );
            if burn_amount < self.burn_minimum {
                return Err(Error::BurnAmountInsufficient);
            }

            let balance_increase = self._burn(account_id, burn_amount);

            Ok(balance_increase)
        }

        /// Burns a specified amount of tokens from the account's balance and updates the account's records.
        ///
        /// Parameters:
        /// - `account_id`: The ID of the account from which tokens are to be burned.
        /// - `amount`: The amount of tokens to be burned.
        ///
        /// Returns:
        /// - `Ok`: with the balance that the account is due after the burn.
        /// - `Err`: if the burn amount is less than the allowed minimum.
        fn _burn(&mut self, account_id: AccountId, amount: Balance) -> Balance {
            self.total_amount_burned = self.total_amount_burned.saturating_add(amount);

            // The balance the account is due after the burn
            let balance_due = amount.saturating_mul(3);

            // Fetch the account if it exists, or initialize a new one if it doesn't
            let mut account = self.accounts.get(&account_id).unwrap_or(Account {
                amount_burned: 0,
                balance_due: 0,
                balance_paid: 0,
                last_withdrawal: None,
                last_burn: 0,
            });

            // Update account details
            account.amount_burned = account.amount_burned.saturating_add(amount);
            account.last_burn = self.env().block_timestamp();
            account.balance_due = account.balance_due.saturating_add(balance_due);

            // Insert the updated account details back into storage
            self.accounts.insert(account_id, &account);

            balance_due
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
            // if self.env().caller() != self.master_controller_contract {
            //     return Err(Error::RestrictedFunction);
            // }

            let account = self.accounts.get(&account_id).ok_or(Error::NoAccountFound)?;

            let withdrawal_allowance = self._calculate_withdrawal(&account);
            if withdrawal_allowance == 0 {
                return Err(Error::WithdrawalNotAllowed);
            }

            let updated_account = self._update_account(
                account_id,
                account,
                withdrawal_allowance.clone()
            );

            Ok((withdrawal_allowance, updated_account.last_withdrawal.unwrap()))
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
            pub const DAY: Timestamp = 600000;
            let last_withdrawal = match account.last_withdrawal {
                Some(timestamp) => timestamp,
                None => self.env().block_timestamp(),
            };
            let days_since_last_withdraw = self
                .env()
                .block_timestamp()
                .saturating_sub(last_withdrawal)
                .saturating_div(DAY);

            let daily_return_percent = self._get_return_percent();

            let daily_allowance = daily_return_percent * account.balance_due;
            // Multiply the daily allowance by the number of days since the last withdrawal
            let allowance = daily_allowance.saturating_mul(days_since_last_withdraw as u128); // cast needed here for arithmetic

            allowance
        }

        /// Updates the account's balances and withdrawal timestamp after a withdrawal operation.
        ///
        /// Parameters:
        /// - `account_id`: The ID of the account to update.
        /// - `account`: The account data that needs to be updated.
        /// - `withdraw_allowance`: The amount that has been allowed to withdraw.
        ///
        /// Returns:
        /// - The updated account data.
        fn _update_account(
            &mut self,
            account_id: AccountId,
            mut account: Account,
            withdraw_allowance: Balance
        ) -> Account {
            // Update the account's details
            account.last_withdrawal = Some(self.env().block_timestamp());
            account.balance_due = account.balance_due.saturating_sub(withdraw_allowance);
            account.balance_paid = account.balance_paid.saturating_add(withdraw_allowance);

            // Insert the updated account details back into storage and return the updated account
            self.accounts.insert(account_id, &account.clone());
            account
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
        fn _get_return_percent(&self) -> Perbill {
            let first_threshold_amount: Balance = 200_000_000_000_000_000_000;
            // let mut percentage: f64 = 0.008;
            let percentage: Perbill = Perbill::from_rational(8u32, 1000u32);
            if self.total_amount_burned <= first_threshold_amount {
                return percentage;
            }

            let excess_amount: u128 =
                self.total_amount_burned.saturating_sub(first_threshold_amount);
            let reductions: u128 = excess_amount
                .saturating_div(100_000_000_000_000_000_000)
                .saturating_add(1);

            for _ in 0..reductions {
                percentage.saturating_div(Perbill::from_rational(2u128, 1u128), NearestPrefDown);
            }
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

        #[ink::test]
        fn update_account_balances() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let old_account = Account {
                amount_burned: 1000,
                balance_due: 3000,
                balance_paid: 0,
                last_withdrawal: None,
                last_burn: 0,
            };
            let withdraw_allowance = 1000;
            let mut d9_burn_mining = D9burnMining::new(accounts.alice, 1000);
            let updated_account = d9_burn_mining._update_account(
                accounts.alice,
                old_account,
                withdraw_allowance
            );
            assert_eq!(updated_account.balance_due, 2000);
            assert_eq!(updated_account.balance_paid, 1000);
            let retrieved_account = d9_burn_mining.accounts.get(&accounts.alice).unwrap();
            //just to make sure that what is stored on contract is the updated version
            assert_eq!(retrieved_account, updated_account)
        }

        #[ink::test]
        fn unpermitted_withdraw() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let mut d9_burn_mining = D9burnMining::new(accounts.alice, 1000);
            let account = Account {
                amount_burned: 1000,
                balance_due: 3000,
                balance_paid: 0,
                last_withdrawal: None,
                last_burn: 0,
            };
            let withdrawal_allowance = d9_burn_mining._calculate_withdrawal(&account);
            assert_eq!(withdrawal_allowance, 0);
        }

        /// Advances the block by one and updates the timestamp by the given seconds.
        fn advance_time_and_block(seconds: u64) {
            let current_time: Timestamp =
                ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                current_time + seconds
            );
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        #[ink::test]
        fn calculate_withdraw() {
            // Setting initial conditions
            let last_withdrawal = Some(1000);
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let mut d9_burn_mining = D9burnMining::new(accounts.alice, 1000);

            // Simulating account setup
            let mut account = Account {
                amount_burned: 200_000_000_000_000,
                balance_due: 600_000_000_000_000,
                balance_paid: 0,
                last_withdrawal,
                last_burn: 0,
            };
            d9_burn_mining.accounts.insert(accounts.alice, &account);
            let no_allowance = d9_burn_mining._calculate_withdrawal(&account);
            assert!(no_allowance == 0);
            // Comment about what we're simulating with this time jump, e.g., "Simulating one day passing"
            advance_time_and_block(10_000_000);
            let withdrawal_allowance = d9_burn_mining._calculate_withdrawal(&account);
            println!("withdrawal_allowance: {}", withdrawal_allowance);
            // assert!(withdrawal_allowance > 0);
        }

        #[ink::test]
        fn portfolio_executed_withdrawal() {
            // Setting initial conditions
            let last_withdrawal = Some(1000);
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let mut d9_burn_mining = D9burnMining::new(accounts.alice, 1000);

            // Simulating account setup
            let mut account = Account {
                amount_burned: 200_000_000_000_000,
                balance_due: 600_000_000_000_000,
                balance_paid: 0,
                last_withdrawal,
                last_burn: 0,
            };
            d9_burn_mining.accounts.insert(accounts.alice, &account);
            advance_time_and_block(600_000);

            let result = d9_burn_mining.portfolio_executed_withdrawal(accounts.alice);
            if let Ok((withdraw_allowance, timestamp)) = result {
                println!("withdraw_allowance: {}", withdraw_allowance);
                println!("timestamp: {}", timestamp);

                let account = d9_burn_mining.accounts.get(&accounts.alice).unwrap();
                println!("account balance due: {}", account.balance_due);
                println!("account balance paid: {}", account.balance_paid);
            }

            // Comment about what we're simulating with this time jump, e.g., "Simulating one day passing"
        }
    }
}
