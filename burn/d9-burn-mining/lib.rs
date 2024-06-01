#![cfg_attr(not(feature = "std"), no_std, no_main)]

use d9_burn_common::{Account, D9Environment, Error};

#[ink::contract(env = D9Environment)]
// #[ink::contract(env = D9Environment)]
pub mod d9_burn_mining {
    use super::*;
    use ink::prelude::vec::Vec;
    use ink::storage::Mapping;
    use sp_arithmetic::Perbill;
    use sp_arithmetic::Perquintill;
    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        pub total_amount_burned: Balance,
        /// the controller of this contract
        main_pool: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
        ///minimum permissible burn amount
        pub burn_minimum: Balance,
        /// set it here to easily adjust for testing for unit, e2e tests and test network
        pub day_milliseconds: Timestamp,
        pub admin: AccountId,
    }

    impl D9burnMining {
        #[ink(constructor, payable)]
        pub fn new(main_pool: AccountId, burn_minimum: Balance) -> Self {
            let day_milliseconds: Timestamp = 86_400_000;
            Self {
                total_amount_burned: Default::default(),
                main_pool,
                accounts: Default::default(),
                burn_minimum,
                day_milliseconds,
                admin: Self::env().caller(),
            }
        }

        #[ink(message)]
        pub fn change_main(&mut self, new_main: AccountId) -> Result<(), Error> {
            if self.env().caller() != self.admin {
                return Err(Error::RestrictedFunction);
            }
            self.main_pool = new_main;
            Ok(())
        }
        #[ink(message)]
        pub fn get_total_burned(&self) -> Balance {
            self.total_amount_burned
        }

        #[ink(message)]
        pub fn set_day_milliseconds(
            &mut self,
            new_day_milliseconds: Timestamp,
        ) -> Result<(), Error> {
            if self.env().caller() != self.admin {
                return Err(Error::RestrictedFunction);
            }
            self.day_milliseconds = new_day_milliseconds;
            Ok(())
        }

        #[ink(message)]
        pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
            self.accounts.get(&account_id)
        }

        /// burn funcion callable by ownly master contract
        ///
        /// does the necessary checks then calls the internal burn function `_burn`
        #[ink(message)]
        pub fn initiate_burn(
            &mut self,
            account_id: AccountId,
            burn_amount: Balance,
        ) -> Result<Balance, Error> {
            if self.env().caller() != self.main_pool {
                return Err(Error::RestrictedFunction);
            }
            if burn_amount < self.burn_minimum {
                return Err(Error::BurnAmountInsufficient);
            }

            if burn_amount % 100 != 0 {
                return Err(Error::MustBeMultipleOf100);
            }

            let balance_increase = self._burn(account_id, burn_amount);

            Ok(balance_increase)
        }

        /// executes burn function and updates internal state
        fn _burn(&mut self, account_id: AccountId, amount: Balance) -> Balance {
            self.total_amount_burned = self.total_amount_burned.saturating_add(amount);
            // The balance the account is due after the burn
            let balance_due = amount.saturating_mul(3);
            // Fetch the account if it exists, or initialize a new one if it doesn't
            let mut account = self
                .accounts
                .get(&account_id)
                .unwrap_or(Account::new(self.env().block_timestamp()));
            // Update account details
            account.amount_burned = account.amount_burned.saturating_add(amount);
            let new_time = self.env().block_timestamp();
            account.last_burn = new_time.clone();
            account.last_interaction = new_time;
            account.balance_due = account.balance_due.saturating_add(balance_due);

            // Insert the updated account details back into storage
            self.accounts.insert(account_id, &account);

            balance_due
        }

        /// calculate values to be used by the burn manager
        #[ink(message)]
        pub fn prepare_withdrawal(
            &mut self,
            account_id: AccountId,
        ) -> Result<(Balance, Timestamp), Error> {
            if self.env().caller() != self.main_pool {
                return Err(Error::RestrictedFunction);
            }

            let mut account = self
                .accounts
                .get(&account_id)
                .ok_or(Error::NoAccountFound)?;

            let base_extraction = self._calculate_base_extraction(&account);
            if base_extraction == 0 {
                return Err(Error::WithdrawalNotAllowed);
            }

            let referral_boost =
                self._calculate_referral_boost_reward(account.referral_boost_coefficients);

            let total_withdrawal = base_extraction.saturating_add(referral_boost);

            // Update the account's details
            let new_time = self.env().block_timestamp();
            account.last_withdrawal = Some(new_time.clone());
            account.last_interaction = new_time;
            let old_balance_due = account.balance_due;
            account.balance_due = account.balance_due.saturating_sub(total_withdrawal);
            account.balance_paid = account.balance_paid.saturating_add(total_withdrawal);
            account.referral_boost_coefficients = (0, 0);

            // Insert the updated account details back into storage and return the updated account
            self.accounts.insert(account_id, &account.clone());
            let maybe_ancestors = self.get_ancestors(account_id);
            if maybe_ancestors.is_some() {
                let ancestors = maybe_ancestors.unwrap();
                self._update_ancestors_coefficents(base_extraction, &ancestors);
            }
            {
                if total_withdrawal > old_balance_due {
                    Ok((old_balance_due, account.last_withdrawal.unwrap()))
                } else {
                    Ok((total_withdrawal, account.last_withdrawal.unwrap()))
                }
            }
        }

        #[ink(message)]
        pub fn get_ancestors(&self, account_id: AccountId) -> Option<Vec<AccountId>> {
            let result = self.env().extension().get_ancestors(account_id);
            match result {
                Ok(ancestors) => ancestors,
                Err(_) => None,
            }
        }

        #[ink(message)]
        pub fn update_data(
            &mut self,
            user: AccountId,
            amount_burned: Balance,
        ) -> Result<(), Error> {
            if self.env().caller() != self.main_pool {
                return Err(Error::RestrictedFunction);
            }
            let mut account = self.accounts.get(&user).ok_or(Error::NoAccountFound)?;
            self.total_amount_burned = self
                .total_amount_burned
                .saturating_sub(account.amount_burned);
            account.amount_burned = amount_burned;
            account.balance_due = amount_burned.saturating_mul(3);
            self.accounts.insert(user, &account);
            self.total_amount_burned = self.total_amount_burned.saturating_add(amount_burned);
            Ok(())
        }

        /// Modifies the code which is used to execute calls to this contract address (`AccountId`).
        ///
        /// We use this to upgrade the contract logic. We don't do any authorization here, any caller
        /// can execute this method. In a production contract you would do some authorization here.
        #[ink(message)]
        pub fn set_code(&mut self, code_hash: [u8; 32]) {
            let caller = self.env().caller();
            assert!(caller == self.admin, "Only admin can set code hash.");
            ink::env::set_code_hash(&code_hash).unwrap_or_else(|err| {
                panic!(
                    "Failed to `set_code_hash` to {:?} due to {:?}",
                    code_hash, err
                )
            });
            ink::env::debug_println!("Switched code hash to {:?}.", code_hash);
        }

        /// Calculates the allowed withdrawal amount for an account.
        ///
        /// Factors in the time since the last withdrawal and daily return percentage.
        /// Returns the computed allowance.
        fn _calculate_base_extraction(&self, account: &Account) -> Balance {
            let last_interaction = account.last_interaction;

            let days_since_last_action = self
                .env()
                .block_timestamp()
                .saturating_sub(last_interaction)
                .saturating_div(self.day_milliseconds);

            let daily_return_percent: Perquintill = self.get_return_percent();

            // let daily_allowance = daily_return_percent * account.balance_due;
            let daily_allowance = daily_return_percent.mul_floor(account.amount_burned);
            // Multiply the daily allowance by the number of days since the last withdrawal
            let allowance = daily_allowance.saturating_mul(days_since_last_action as u128); // cast needed here for arithmetic

            {
                if allowance > account.balance_due {
                    return account.balance_due;
                } else {
                    return allowance;
                }
            }
        }

        fn _calculate_referral_boost_reward(
            &self,
            referral_coefficients: (Balance, Balance),
        ) -> Balance {
            let direct_referral_boost =
                Perbill::from_percent(10).mul_floor(referral_coefficients.0);
            let indirect_referral_boost =
                Perbill::from_percent(1).mul_floor(referral_coefficients.1);

            direct_referral_boost.saturating_add(indirect_referral_boost)
        }
        //todo what is last_burn used for
        fn _update_ancestors_coefficents(&mut self, allowance: Balance, ancestors: &[AccountId]) {
            let parent = ancestors[0];
            let mut account = self
                .accounts
                .get(&parent)
                .unwrap_or_else(|| Account::new(self.env().block_timestamp()));
            // add allowance to parent's x coefficient in R_a = base_extraction_rate + x0.1 + y0.01
            account.referral_boost_coefficients.0 = account
                .referral_boost_coefficients
                .0
                .saturating_add(allowance);
            self.accounts.insert(parent, &account);

            for ancestor in ancestors.iter().skip(1) {
                let mut ancestor_account = self
                    .accounts
                    .get(&ancestor)
                    .unwrap_or_else(|| Account::new(self.env().block_timestamp()));
                // add allowance to ancestor's y coefficient in R_a = base_extraction_rate + x0.1 + y0.01
                ancestor_account.referral_boost_coefficients.1 = ancestor_account
                    .referral_boost_coefficients
                    .1
                    .saturating_add(allowance);
                self.accounts.insert(ancestor, &ancestor_account);
            }
        }

        #[ink(message)]
        pub fn get_return_percent(&self) -> Perquintill {
            let first_threshold_amount: Balance = 200_000_000_000_000_000_000;
            // let mut percentage: f64 = 0.008;
            let percentage: Perquintill = Perquintill::from_rational(8u64, 1000u64);
            if self.total_amount_burned <= first_threshold_amount {
                return percentage;
            }

            let excess_amount: u128 = self
                .total_amount_burned
                .saturating_sub(first_threshold_amount);
            let reductions: u128 = excess_amount
                .saturating_div(100_000_000_000_000_000_000)
                .saturating_add(1);
            let divided_percent_by = Balance::from(2u32).pow(reductions as u32);
            // for _ in 0..reductions {
            //     percentage.saturating_reciprocal_mul(Perbill::from_rational(2u32, 1u32));
            // }
            self.divide_perquintill_by_number(percentage, divided_percent_by as u64)
        }

        fn divide_perquintill_by_number(
            &self,
            perquintill_value: Perquintill,
            divisor: u64,
        ) -> Perquintill {
            if divisor == 0 {
                panic!("Division by zero is not allowed");
            }
            let divided_value = perquintill_value.deconstruct().saturating_div(divisor);

            // Create a new Perbill instance from the divided value
            Perquintill::from_parts(divided_value)
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;
        use ink::env::block_timestamp;
        fn set_block_time(init_time: Timestamp) {
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(init_time);
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        ///moves block forward by `move_forward_by` in milliseconds and moves chain forwards by one block
        fn move_time_forward(move_forward_by: Timestamp) {
            let current_block_time: Timestamp =
                ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            let _ = ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                current_block_time + move_forward_by,
            );
            let _ = ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }
        #[ink::test]
        fn cant_withdraw_early() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            static BURN_MINIMUM: Balance = 100_000_000_000_000;
            let d9_burn_mining = D9burnMining::new(accounts.alice, BURN_MINIMUM);
            static INITIAL_TIME: Timestamp = 1672531200000;
            set_block_time(INITIAL_TIME);
            let account = Account::new(INITIAL_TIME + 1);

            let withdrawal_allowance = d9_burn_mining._calculate_base_extraction(&account);
            assert_eq!(withdrawal_allowance, 0);
        }
        #[ink::test]
        fn get_proper_percentage() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            static BURN_MINIMUM: Balance = 100_000_000_000_000;
            let mut d9_burn_mining = D9burnMining::new(accounts.alice, BURN_MINIMUM);
            static INITIAL_TIME: Timestamp = 1672531200000;
            set_block_time(INITIAL_TIME);
            d9_burn_mining.total_amount_burned = 200_000_000_000_000_000_000;
            let percentage = d9_burn_mining.get_return_percent();
            assert_eq!(percentage, Perbill::from_rational(8u32, 1000u32));
            d9_burn_mining.total_amount_burned = 250_000_000_000_000_000_000;
            let smaller_percentage = d9_burn_mining.get_return_percent();
            assert_eq!(smaller_percentage, Perbill::from_rational(4u32, 1000u32));
            d9_burn_mining.total_amount_burned = 350_000_000_000_000_000_000;
            let even_smaller_percentage = d9_burn_mining.get_return_percent();
            assert_eq!(
                even_smaller_percentage,
                Perbill::from_rational(2u32, 1000u32)
            );
        }
        #[ink::test]
        fn withdrawal_permitted() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            static BURN_MINIMUM: Balance = 100_000_000_000_000;
            let d9_burn_mining = D9burnMining::new(accounts.alice, BURN_MINIMUM);
            static INITIAL_TIME: Timestamp = 1672531200000;
            set_block_time(INITIAL_TIME);

            let mut account = Account::new(INITIAL_TIME);
            account.amount_burned = 1_000_000_000_000_000;
            account.balance_due = 3_000_000_000_000_000;
            move_time_forward(2 * d9_burn_mining.day_milliseconds);
            let withdrawal_allowance = d9_burn_mining._calculate_base_extraction(&account);
            assert_ne!(withdrawal_allowance, 0);
        }
        #[ink::test]
        fn correct_withdrawal_amount() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            static BURN_MINIMUM: Balance = 100_000_000_000_000;
            let d9_burn_mining = D9burnMining::new(accounts.alice, BURN_MINIMUM);
            static INITIAL_TIME: Timestamp = 1672531200000;
            set_block_time(INITIAL_TIME);
            let mut account = Account::new(INITIAL_TIME);
            account.amount_burned = 1_000_000_000_000_000;
            account.balance_due = 3_000_000_000_000_000;
            move_time_forward(2 * d9_burn_mining.day_milliseconds);
            let withdrawal_allowance = d9_burn_mining._calculate_base_extraction(&account);
            assert_ne!(withdrawal_allowance, 48000000000000);
        }

        #[ink::test]
        fn _calculate_base_with_referral_boost() {
            let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            static BURN_MINIMUM: Balance = 100_000_000_000_000;
            let d9_burn_mining = D9burnMining::new(accounts.alice, BURN_MINIMUM);
            static INITIAL_TIME: Timestamp = 1672531200000;
            set_block_time(INITIAL_TIME);
            let current_timestamp = block_timestamp::<ink::env::DefaultEnvironment>();
            println!("current_timestamp: {}", current_timestamp);
            let mut account = Account::new(INITIAL_TIME);
            account.amount_burned = 1_000_000_000_000_000;
            account.balance_due = 3_000_000_000_000_000;
            account.referral_boost_coefficients = (1_000_000_000_000_000, 1_000_000_000_000_000);
            move_time_forward(d9_burn_mining.day_milliseconds);

            let base_extraction = d9_burn_mining._calculate_base_extraction(&account);
            let referral_boost = d9_burn_mining
                ._calculate_referral_boost_reward(account.referral_boost_coefficients);
            let total = base_extraction.saturating_add(referral_boost);
            assert_eq!(
                total,
                24_000000000000 + 100_000_000_000_000 + 10_000_000_000_000
            );
        }
    }
}
