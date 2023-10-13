#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod d9_merchant_mining {
    use scale::{ Decode, Encode };
    pub use d9_chain_extension::D9Environment;
    use ink::storage::Mapping;
    use sp_arithmetic::Percent;
    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
    )]
    pub struct Account {
        green_points: Balance,
        last_conversion: Timestamp,
        redeemed_usdt: Balance,
        redeemed_d9: Balance,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        InsufficientPayment,
        NoMerchantAccountFound,
        NoAccountFound,
        NothingToRedeem,
    }

    #[ink(event)]
    pub struct Subscription {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        expiry: Timestamp,
    }

    #[ink(storage)]
    pub struct D9MerchantMining {
        /// accountId to mercchat account expiry date
        merchant_expiry: Mapping<AccountId, Timestamp>,
        /// rewards system accounts
        accounts: Mapping<AccountId, Account>,
        subscription_fee: Balance,
        main_contract: AccountId,
        milliseconds_day: Timestamp,
    }

    impl D9MerchantMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(subscription_fee: Balance, main_contract: AccountId) -> Self {
            Self {
                main_contract,
                merchant_expiry: Default::default(),
                accounts: Default::default(),
                subscription_fee,
                milliseconds_day: 86400000,
            }
        }

        #[ink(message, payable)]
        pub fn give_green_points(&mut self, account_id: AccountId) -> Result<Balance, Error> {
            let amount = self.env().transferred_value();
            let mut account = self.accounts.get(&account_id).unwrap();
            account.green_points = account.green_points.saturating_add(amount.saturating_mul(100));
            self.accounts.insert(account_id, &account);
            Ok(account.green_points)
        }

        //with draw a certain amount of d9 that has been converted into red points
        #[ink(message)]
        pub fn redeem_d9(&mut self) -> Result<(), Error> {
            let account = self.env().caller();
            let maybe_account = self.accounts.get(&account);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let mut account = maybe_account.unwrap();
            let red_points = self.calculate_red_points(
                account.green_points,
                account.last_conversion
            );
            if red_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            let redeemable_d9 = red_points.saturating_div(100);
            account.green_points = account.green_points.saturating_sub(red_points);
            account.redeemed_d9 = account.redeemed_d9.saturating_add(redeemable_d9);
            account.last_conversion = self.env().block_timestamp();
            self.env().transfer(self.env().caller(), redeemable_d9).expect("Transfer failed");
            self.accounts.insert(self.env().caller(), &account);
            Ok(())
        }

        fn calculate_red_points(&self, amount: Balance, last_timestamp: Timestamp) -> Balance {
            let accrue_rate = Percent::from_rational(5u128, 100000u128);

            let days_since_last_redeem = self
                .env()
                .block_timestamp()
                .saturating_sub(last_timestamp)
                .saturating_div(self.milliseconds_day) as Balance;

            let red_points = accrue_rate
                .saturating_reciprocal_mul(amount)
                .saturating_mul(days_since_last_redeem);

            red_points
        }

        #[ink(message)]
        pub fn get_expiry(&self, account_id: AccountId) -> Result<Timestamp, Error> {
            let expiry = self.merchant_expiry.get(&account_id);
            match expiry {
                Some(expiry) => Ok(expiry),
                None => Err(Error::NoMerchantAccountFound),
            }
        }

        #[ink(message, payable)]
        pub fn d9_subscribe(&mut self) -> Result<Timestamp, Error> {
            let amount_in_base = self.env().transferred_value();
            let account_id = self.env().caller();
            let update_expiry_result = self.update_subscription(account_id, amount_in_base);
            update_expiry_result
        }

        /// Updates the subscription expiry date for a given account based on the payment provided.
        ///
        /// This function calculates the new expiry date of a subscription based on the `amount_in_base`
        /// provided. The amount is divided by the subscription fee to determine how many months
        /// the subscription should be extended.
        ///
        /// # Parameters
        /// - `account_id`: The account ID for which the subscription needs to be updated.
        /// - `amount_in_base`: The amount paid for the subscription. This amount is used to
        ///   calculate how many months the subscription will be extended.
        ///
        /// # Returns
        /// - `Ok(Timestamp)`: The new expiry timestamp for the subscription if the update is successful.
        /// - `Err(Error::InsufficientPayment)`: If the `amount_in_base` provided is not enough
        ///   to cover at least one month of subscription.
        ///
        /// # Emits
        /// - `Subscription`: An event with the account ID and the new expiry date.
        ///
        /// # Note
        /// - The function will set the expiry to the current block timestamp if the existing
        ///   expiry date is more than a month in the past. Otherwise, it extends the current expiry date.
        /// - One month is considered to be `2629800000` units of time.
        fn update_subscription(
            &mut self,
            account_id: AccountId,
            amount_in_base: Balance
        ) -> Result<Timestamp, Error> {
            let months = amount_in_base.saturating_div(self.subscription_fee) as Timestamp;
            if months == 0 {
                return Err(Error::InsufficientPayment);
            }
            let one_month: Timestamp = 2629800000;
            let current_expiry: Timestamp = match self.merchant_expiry.get(&account_id) {
                Some(expiry) => {
                    if expiry < self.env().block_timestamp().saturating_sub(one_month) {
                        self.env().block_timestamp()
                    } else {
                        expiry
                    }
                }
                None => self.env().block_timestamp(),
            };
            let new_expiry = current_expiry.saturating_add(months.saturating_mul(one_month));
            self.merchant_expiry.insert(account_id.clone(), &new_expiry);
            self.env().emit_event(Subscription {
                account_id,
                expiry: new_expiry,
            });
            Ok(new_expiry)
        }

        //   fn get_current_rate(&self) -> u128;
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;
        ///insufficient subscription fee.
        #[test]
        fn subscription_fail_insufficient_payment() {
            let subscription_fee: Balance = 10_000_000;
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let mut contract = D9MerchantMining::new(subscription_fee);
            let below_minimum: Balance = 1_000_000;
            let result = contract.update_subscription(default_accounts.alice, below_minimum);
            assert_eq!(result, Err(Error::InsufficientPayment));
        }

        ///user gets a new subscription month
        #[ink::test]
        fn create_new_subscription() {
            let subscription_fee: Balance = 10_000_000;
            let one_month: Timestamp = 2629800000; //in millieseconds
            let payment_amount: Balance = 10_000_000;
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let mut contract = D9MerchantMining::new(subscription_fee);
            let contract_address = ink::env::account_id::<ink::env::DefaultEnvironment>();
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(default_accounts.alice);
            ink::env::test::set_callee::<ink::env::DefaultEnvironment>(contract_address);
            let current_block_time: Timestamp =
                ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            let result = contract.update_subscription(default_accounts.alice, payment_amount);
            assert_eq!(result, Ok(current_block_time + one_month));
        }

        #[ink::test]
        fn update_expired_subscription() {
            let one_month: Timestamp = 2629800000; //in millieseconds
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let contract_address: AccountId =
                ink::env::account_id::<ink::env::DefaultEnvironment>();
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(default_accounts.alice);
            ink::env::test::set_callee::<ink::env::DefaultEnvironment>(contract_address);
            let janurary_first_2023: Timestamp = 1672545661000; // 4:01:01 AM GMT+8
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                janurary_first_2023
            );
            let mut contract = D9MerchantMining::new(10_000_000);
            //create a new subscription
            let _ = contract.update_subscription(default_accounts.alice, 10_000_000);
            let merchant_expiry = contract.get_expiry(default_accounts.alice);
            // new block starts here
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                janurary_first_2023 + 2 * one_month
            );
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
            let new_block_time = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            assert!(merchant_expiry.unwrap() < new_block_time);
            let result = contract.update_subscription(default_accounts.alice, 10_000_000);
            assert_eq!(result, Ok(new_block_time + one_month));
        }

        #[ink::test]
        fn udpate_unexpired_subscription() {
            let one_month: Timestamp = 2629800000; //in millieseconds
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            let contract_address: AccountId =
                ink::env::account_id::<ink::env::DefaultEnvironment>();
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(default_accounts.alice);
            ink::env::test::set_callee::<ink::env::DefaultEnvironment>(contract_address);
            let janurary_first_2023: Timestamp = 1672545661000; // 4:01:01 AM GMT+8
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                janurary_first_2023
            );
            let mut contract = D9MerchantMining::new(10_000_000);
            //create a new subscription
            let _ = contract.update_subscription(default_accounts.alice, 10_000_000);
            let merchant_expiry = contract.get_expiry(default_accounts.alice);
            // new block starts here
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                janurary_first_2023.saturating_add(one_month.saturating_div(2))
            );
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
            let new_block_time = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            assert!(merchant_expiry.unwrap().clone() > new_block_time);
            let result = contract.update_subscription(default_accounts.alice, 10_000_000);
            assert_eq!(result, Ok(merchant_expiry.unwrap() + one_month));
        }
    }

    /// This is how you'd write end-to-end (E2E) or integration tests for ink! contracts.
    ///
    /// When running these you need to make sure that you:
    /// - Compile the tests with the `e2e-tests` feature flag enabled (`--features e2e-tests`)
    /// - Are running a Substrate node which contains `pallet-contracts` in the background
    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;
        /// A helper function used for calling contract messages.
        use ink_e2e::build_message;

        /// The End-to-End test `Result` type.
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        /// We test that we can upload and instantiate the contract using its default constructor.
        #[ink_e2e::test]
        async fn default_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = D9ConsumerMiningRef::default();

            // When
            let contract_account_id = client
                .instantiate("d9_consumer_mining", &ink_e2e::alice(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            // Then
            let get = build_message::<D9ConsumerMiningRef>(contract_account_id.clone()).call(
                |d9_consumer_mining| d9_consumer_mining.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::alice(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            Ok(())
        }

        /// We test that we can read and write a value from the on-chain contract contract.
        #[ink_e2e::test]
        async fn it_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = D9ConsumerMiningRef::new(false);
            let contract_account_id = client
                .instantiate("d9_consumer_mining", &ink_e2e::bob(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            let get = build_message::<D9ConsumerMiningRef>(contract_account_id.clone()).call(
                |d9_consumer_mining| d9_consumer_mining.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            // When
            let flip = build_message::<D9ConsumerMiningRef>(contract_account_id.clone()).call(
                |d9_consumer_mining| d9_consumer_mining.flip()
            );
            let _flip_result = client
                .call(&ink_e2e::bob(), flip, 0, None).await
                .expect("flip failed");

            // Then
            let get = build_message::<D9ConsumerMiningRef>(contract_account_id.clone()).call(
                |d9_consumer_mining| d9_consumer_mining.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), true));

            Ok(())
        }
    }
}
