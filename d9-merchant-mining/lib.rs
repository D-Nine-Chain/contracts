#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod d9_merchant_mining {
    use super::*;
    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use sp_arithmetic::Perbill;
    use ink::prelude::vec::Vec;
    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
    )]
    pub struct Account {
        //green points => red points  Δ =  0.005 % / day
        green_points: Balance,
        //timestamp of last conversion to d9 or usdt
        last_conversion: Option<Timestamp>,
        //red points => usdt
        redeemed_usdt: Balance,
        //red points => d9
        redeemed_d9: Balance,
        // date of creation
        created_at: Timestamp,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        InsufficientPayment,
        NoMerchantAccountFound,
        MerchantAccountExpired,
        NoAccountFound,
        NothingToRedeem,
        ErrorTransferringToMainContract,
        TransferFailed,
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
        /// rewards system accounts
        merchant_expiry: Mapping<AccountId, Timestamp>,
        accounts: Mapping<AccountId, Account>,
        subscription_fee: Balance,
        main_contract: AccountId,
        milliseconds_day: Timestamp,
    }

    impl D9MerchantMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(subscription_fee: Balance, main_contract: AccountId) -> Self {
            if subscription_fee == 0 {
                panic!("subscription fee cannot be zero");
            }
            Self {
                main_contract,
                merchant_expiry: Default::default(),
                accounts: Default::default(),
                subscription_fee,
                milliseconds_day: 86_400_000,
            }
        }

        /// create merchant account subscription
        #[ink(message, payable)]
        pub fn d9_subscribe(&mut self) -> Result<Timestamp, Error> {
            let amount_in_base = self.env().transferred_value();
            let account_id = self.env().caller();
            let update_expiry_result = self.update_subscription(account_id, amount_in_base);
            update_expiry_result
        }

        /// pay green points to `account_id` using d9
        #[ink(message, payable)]
        pub fn give_green_points(
            &mut self,
            account_id: AccountId
        ) -> Result<(AccountId, Balance), Error> {
            //check if valid merchant subscription
            let caller: AccountId = self.env().caller();
            let d9_to_convert: Balance = self.env().transferred_value();
            // let d9_to_convert = 10000;
            let merchant_expiry_option: Option<Timestamp> = self.merchant_expiry.get(&caller);
            match merchant_expiry_option {
                Some(expiry) => {
                    if expiry < self.env().block_timestamp() {
                        return Err(Error::MerchantAccountExpired);
                    }
                }
                None => {
                    return Err(Error::NoMerchantAccountFound);
                }
            }

            //convert to green points
            if d9_to_convert == 0 {
                return Err(Error::InsufficientPayment);
            }
            let maybe_account: Option<Account> = self.accounts.get(&account_id);
            let mut account: Account = match maybe_account {
                Some(account) => account,
                None =>
                    Account {
                        green_points: 0,
                        last_conversion: None,
                        redeemed_usdt: 0,
                        redeemed_d9: 0,
                        created_at: self.env().block_timestamp(),
                    },
            };

            account.green_points = account.green_points.saturating_add(
                d9_to_convert.saturating_mul(100)
            );
            self.accounts.insert(account_id, &account);
            // let transfer_result = self.env().transfer(self.main_contract, amount);
            // if transfer_result.is_err() {
            //     return Err(Error::ErrorTransferringToMainContract);
            // }
            Ok((account_id, account.green_points))
        }

        ///withdraw a certain amount of d9 that has been converted into red points
        #[ink(message)]
        pub fn redeem_d9(&mut self) -> Result<Balance, Error> {
            //get account
            let caller = self.env().caller();
            let maybe_account = self.accounts.get(&caller);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let mut account = maybe_account.unwrap();

            //caculate green => red points conversion
            let last_redeem_timestamp = account.last_conversion.unwrap_or(account.created_at);
            let red_points = self.calculate_red_points(account.green_points, last_redeem_timestamp);
            if red_points == 0 {
                return Err(Error::NothingToRedeem);
            }

            //calculated red points => d9 conversion
            // let redeemable_d9 = red_points.saturating_div(100);
            let redeemable_d9 = red_points;

            //attempt to pay ancestors
            if let Some(ancestors) = self.get_ancestors(caller) {
                let remainder = self.pay_ancestors(redeemable_d9, &ancestors)?;
                account.green_points = account.green_points.saturating_sub(red_points);
                account.redeemed_d9 = account.redeemed_d9.saturating_add(redeemable_d9);
                account.last_conversion = Some(self.env().block_timestamp());
                self.accounts.insert(caller, &account);
                self.env()
                    .transfer(self.env().caller(), remainder.clone())
                    .expect("Transfer failed");
                return Ok(remainder.clone());
            }

            //update account
            account.green_points = account.green_points.saturating_sub(red_points);
            account.redeemed_d9 = account.redeemed_d9.saturating_add(redeemable_d9);
            account.last_conversion = Some(self.env().block_timestamp());
            self.env()
                .transfer(self.env().caller(), redeemable_d9.clone())
                .expect("Transfer failed");
            self.accounts.insert(caller, &account);

            Ok(redeemable_d9)
        }

        #[ink(message)]
        pub fn get_expiry(&self, account_id: AccountId) -> Result<Timestamp, Error> {
            let expiry = self.merchant_expiry.get(&account_id);
            match expiry {
                Some(expiry) => Ok(expiry),
                None => Err(Error::NoMerchantAccountFound),
            }
        }

        #[ink(message)]
        /// get account details
        pub fn get_account(&self, account_id: AccountId) -> Result<Account, Error> {
            let maybe_account = self.accounts.get(&account_id);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let account = maybe_account.unwrap();
            Ok(account)
        }

        /// redpoints are calculated at the time of withdraw request
        ///
        /// 1 red point = 1 green point
        fn calculate_red_points(
            &self,
            green_points: Balance,
            last_redeem_timestamp: Timestamp
        ) -> Balance {
            // rate green points => red points
            let transmutation_rate = Perbill::from_rational(1u32, 20000u32);

            let days_since_last_redeem = self
                .env()
                .block_timestamp()
                .saturating_sub(last_redeem_timestamp)
                .saturating_div(self.milliseconds_day) as Balance;

            let red_points = transmutation_rate
                .mul_ceil(green_points)
                .saturating_mul(days_since_last_redeem);

            if red_points > green_points {
                return green_points;
            } else if days_since_last_redeem > 10 && red_points == 0 && green_points > 0 {
                return green_points;
            } else {
                return red_points;
            }
        }
        ///create/update subscription, returns new expiry `Timestamp` Result
        fn update_subscription(
            &mut self,
            account_id: AccountId,
            amount_in_base: Balance
        ) -> Result<Timestamp, Error> {
            let months = amount_in_base.saturating_div(self.subscription_fee) as Timestamp;
            if months == 0 {
                return Err(Error::InsufficientPayment);
            }
            let one_month: Timestamp = self.milliseconds_day * 30;
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

        pub fn get_ancestors(&self, account_id: AccountId) -> Option<Vec<AccountId>> {
            let result = self.env().extension().get_ancestors(account_id);
            match result {
                Ok(ancestors) => ancestors,
                Err(_) => None,
            }
        }

        fn pay_ancestors(
            &self,
            allowance: Balance,
            ancestors: &[AccountId]
        ) -> Result<Balance, Error> {
            let mut remainder = allowance;

            // Calculate 10% for the parent
            let ten_percent = Perbill::from_percent(10).mul_floor(allowance);
            let parent = ancestors[0];
            self.transfer(parent, ten_percent)?;
            remainder = remainder.saturating_sub(ten_percent);

            // Calculate 1% for the rest of the ancestors
            let one_percent = Perbill::from_percent(1).mul_floor(allowance);
            for ancestor in ancestors.iter().skip(1) {
                self.transfer(*ancestor, one_percent)?;
                remainder = remainder.saturating_sub(one_percent);
            }

            Ok(remainder)
        }
        fn transfer(&self, account_id: AccountId, amount: Balance) -> Result<(), Error> {
            self.env()
                .transfer(account_id, amount)
                .map_err(|_| Error::TransferFailed)
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;
        use ink::env::DefaultEnvironment;
        use ink::env::test::{ set_value_transferred, set_caller };
        static ONE_MONTH_MILLISECONDS: Timestamp = 86_400_000 * 30;

        /// prepare default accounts and contract address for tests
        fn init_accounts() -> ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> {
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
            default_accounts
        }

        /// setup env values for caller/callee
        fn init_calling_env(caller: AccountId) {
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(caller);

            let contract_address = ink::env::account_id::<ink::env::DefaultEnvironment>();
            ink::env::test::set_callee::<ink::env::DefaultEnvironment>(contract_address);
        }

        /// gives everything as default setup
        fn default_setup() -> (
            ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment>,
            D9MerchantMining,
        ) {
            // init accounts
            let default_accounts = init_accounts();

            //build contract
            let subscription_fee: Balance = 10_000_000;
            let contract = D9MerchantMining::new(subscription_fee, default_accounts.bob);
            (default_accounts, contract)
        }

        ///sets blocktime stamp and moves chain forwards by one block
        fn set_block_time(init_time: Timestamp) {
            ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(init_time);
            ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        ///moves block forward by `move_forward_by` in milliseconds and moves chain forwards by one block
        fn move_time_forward(move_forward_by: Timestamp) {
            let current_block_time: Timestamp =
                ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            let _ = ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
                current_block_time + move_forward_by
            );
            let _ = ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        #[ink::test]
        fn subscription_fail_insufficient_payment() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);

            //create subscription
            let below_minimum_payment: Balance = 1_000_000;
            let result = contract.update_subscription(
                default_accounts.alice,
                below_minimum_payment
            );

            assert_eq!(result, Err(Error::InsufficientPayment));
        }

        #[ink::test]
        fn successfully_create_new_subscription() {
            let (default_accounts, mut contract) = default_setup();

            //prep new merchant account
            let new_merchant = default_accounts.alice;
            let payment_amount: Balance = 10_000_000;
            let result = contract.update_subscription(new_merchant, payment_amount);

            let current_block_time = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            assert_eq!(result, Ok(current_block_time + ONE_MONTH_MILLISECONDS));

            let merchant_expiry = contract.get_expiry(new_merchant);
            assert_eq!(merchant_expiry, Ok(current_block_time + ONE_MONTH_MILLISECONDS));
        }

        #[ink::test]
        fn update_expired_subscription() {
            let (default_accounts, mut contract) = default_setup();

            //default time to jan 1
            let janurary_first_2023: Timestamp = 1672545661000; // 4:01:01 AM GMT+8
            set_block_time(janurary_first_2023);

            //create a new subscription
            let one_month_subscription_fee: Balance = 10_000_000;
            let _ = contract.update_subscription(
                default_accounts.alice,
                one_month_subscription_fee
            );

            // move time forward so as to expire subscription
            move_time_forward(2 * ONE_MONTH_MILLISECONDS);

            // renew new subscription
            let new_block_time = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            let _ = contract.update_subscription(
                default_accounts.alice,
                one_month_subscription_fee
            );

            let expiry = contract.get_expiry(default_accounts.alice).unwrap();
            assert_eq!(expiry, new_block_time + ONE_MONTH_MILLISECONDS);
        }

        #[ink::test]
        fn update_unexpired_subscription() {
            let (default_accounts, mut contract) = default_setup();

            //init time
            let janurary_first_2023: Timestamp = 1672545661000; // 4:01:01 AM GMT+8
            set_block_time(janurary_first_2023);

            //setup initial subscription
            let new_merchant = default_accounts.alice;
            let one_month_subscription_fee = 10_000_000;
            let _ = contract.update_subscription(new_merchant, one_month_subscription_fee);

            move_time_forward(ONE_MONTH_MILLISECONDS / 2);

            let _ = contract.update_subscription(new_merchant, one_month_subscription_fee).unwrap();
            let merchant_expiry = contract.get_expiry(new_merchant).unwrap();
            assert_eq!(merchant_expiry, janurary_first_2023 + 2 * ONE_MONTH_MILLISECONDS + 6);
        }

        #[ink::test]
        fn calculate_red_points() {
            let (_, contract) = default_setup();

            // calculate red point
            let last_redeem_timestamp = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
            let one_hundred_days = 100 * contract.milliseconds_day;
            set_block_time(last_redeem_timestamp + one_hundred_days);

            let red_points = contract.calculate_red_points(200_000_000, last_redeem_timestamp);

            assert_eq!(red_points, 1_000_000)
        }

        #[ink::test]
        fn pay_for_subscription() {
            let (default_accounts, mut contract) = default_setup();
            init_calling_env(default_accounts.alice);
            //create subscription
            let payment_amount: Balance = 10_000_000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let subscription_result = contract.d9_subscribe();
            assert!(subscription_result.is_ok());
            //one month
            assert_eq!(subscription_result.unwrap(), 2592000000);

            //check account
            let expiry = contract.get_expiry(default_accounts.alice);
            assert_eq!(expiry, Ok(2592000000));
        }

        #[ink::test]
        fn give_green_points() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);

            //create subscription
            let subscription_amount: Balance = 10_000_000_000_000;
            set_value_transferred::<DefaultEnvironment>(subscription_amount);
            let _ = contract.d9_subscribe();

            //give green points
            let payment_amount: Balance = 10_000_000_000_000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let green_points_result = contract.give_green_points(default_accounts.bob);

            //check
            assert!(green_points_result.is_ok());
            let (account_id, green_points) = green_points_result.unwrap();
            assert_eq!(account_id, default_accounts.bob);
            assert_eq!(green_points, payment_amount * 100);
            let account_result = contract.get_account(default_accounts.bob);
            assert!(account_result.is_ok());
            let account = account_result.unwrap();
            assert_eq!(account.green_points, payment_amount * 100);
        }

        #[ink::test]
        fn redeem_d9() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);

            //create subscription
            let subscription_amount: Balance = 10_000_000;
            set_value_transferred::<DefaultEnvironment>(subscription_amount);
            let _ = contract.d9_subscribe();

            //give green points
            let payment_amount: Balance = 10000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let _ = contract.give_green_points(default_accounts.bob);
            move_time_forward(86_400_000 * 100);

            //redeem d9
            set_caller::<DefaultEnvironment>(default_accounts.bob);
            let redemption_result = contract.redeem_d9();
            println!("green_points_result: {:?}", redemption_result);
            assert!(redemption_result.is_ok());
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
