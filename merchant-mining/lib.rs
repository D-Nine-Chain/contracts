#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod d9_merchant_mining {
    use super::*;
    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use sp_arithmetic::Perbill;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;

    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
    )]
    pub struct Account {
        //green points => red points  Δ =  0.005 % / day
        green_points: Balance,
        // number (n,m) of sons and grandson respectively
        relationship_factors: (Balance, Balance),
        //timestamp of last conversion to d9 or usdt
        last_conversion: Option<Timestamp>,
        //red points => usdt
        redeemed_usdt: Balance,
        //red points => d9
        redeemed_d9: Balance,
        // date of creation
        created_at: Timestamp,
    }

    impl Account {
        fn new(created_at: Timestamp) -> Self {
            Self {
                green_points: Balance::default(),
                relationship_factors: (0, 0),
                last_conversion: None,
                redeemed_usdt: Balance::default(),
                redeemed_d9: Balance::default(),
                created_at,
            }
        }
    }

    // data to return to user
    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
    )]
    pub struct GreenPointsResult {
        merchant: Balance,
        consumer: Balance,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Currency {
        D9,
        USDT,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        InsufficientPayment,
        InsufficientAllowance,
        NoMerchantAccountFound,
        MerchantAccountExpired,
        NoAccountFound,
        NothingToRedeem,
        ErrorTransferringToMainContract,
        ErrorTransferringToUSDTToMerchant,
        UserUSDTBalanceInsufficient,
        D9TransferFailed,
        USDTTransferFailed,
        OnlyAdmin,
        GrantingAllowanceFailed,
        AMMConversionFailed,
    }

    #[ink(event)]
    pub struct SubscriptionCreated {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        expiry: Timestamp,
    }

    // event for creation of green points
    #[ink(event)]
    pub struct GreenPointsTransaction {
        #[ink(topic)]
        merchant: GreenPointsCreated,
        #[ink(topic)]
        consumer: GreenPointsCreated,
    }
    // a struct associated with the GreenPointsTransaction event
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct GreenPointsCreated {
        account_id: AccountId,
        green_points: Balance,
    }

    #[ink(storage)]
    pub struct D9MerchantMining {
        /// accountId to mercchat account expiry date
        /// rewards system accounts
        merchant_expiry: Mapping<AccountId, Timestamp>,
        accounts: Mapping<AccountId, Account>,
        subscription_fee: Balance,
        usdt_contract: AccountId,
        amm_contract: AccountId,
        mining_pool: AccountId,
        milliseconds_day: Timestamp,
        admin: AccountId,
    }

    impl D9MerchantMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(
            amm_contract: AccountId,
            mining_pool: AccountId,
            usdt_contract: AccountId
        ) -> Self {
            Self {
                admin: Self::env().caller(),
                amm_contract,
                usdt_contract,
                mining_pool,
                merchant_expiry: Default::default(),
                accounts: Default::default(),
                subscription_fee: 1000,
                milliseconds_day: 600_000,
            }
        }

        /// create merchant account subscription
        #[ink(message)]
        pub fn subscribe(&mut self, usdt_amount: Balance) -> Result<Timestamp, Error> {
            let merchant_id = self.env().caller();
            if usdt_amount < self.subscription_fee {
                return Err(Error::InsufficientPayment);
            }
            let validate_transfer = self.validate_usdt_transfer(merchant_id, usdt_amount);
            if let Err(e) = validate_transfer {
                return Err(e);
            }
            let receive_usdt_result = self.receive_usdt_from_user(merchant_id, usdt_amount);
            if let Err(e) = receive_usdt_result {
                return Err(e);
            }

            let update_expiry_result = self.update_subscription(merchant_id, usdt_amount);
            update_expiry_result
        }

        ///create/update subscription, returns new expiry `Timestamp` Result
        fn update_subscription(
            &mut self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<Timestamp, Error> {
            let months = amount.saturating_div(self.subscription_fee) as Timestamp;
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
            self.env().emit_event(SubscriptionCreated {
                account_id,
                expiry: new_expiry,
            });
            Ok(new_expiry)
        }

        ///withdraw a certain amount of d9 that has been converted into red points
        #[ink(message)]
        pub fn redeem_usdt(&mut self) -> Result<Balance, Error> {
            //get account
            let caller = self.env().caller();
            let maybe_account = self.accounts.get(&caller);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let mut account = maybe_account.unwrap();
            if account.green_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            //caculate green => red points conversion
            let last_redeem_timestamp = account.last_conversion.unwrap_or(account.created_at);

            let time_based_red_points = self.calculate_red_points_from_time(
                account.green_points,
                last_redeem_timestamp
            );
            let relationship_based_red_points = self.calculate_red_points_from_relationships(
                account.green_points,
                account.relationship_factors
            );

            let total_red_points = time_based_red_points.saturating_add(
                relationship_based_red_points
            );
            if total_red_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            let convertible_red_points = {
                if total_red_points > account.green_points {
                    account.green_points
                } else {
                    total_red_points
                }
            };
            // deduct red points from green points
            account.green_points = account.green_points.saturating_sub(convertible_red_points);

            //calculated red points => d9 conversion
            // let redeemable_d9 = red_points.saturating_div(100);
            let redeemable_usdt = convertible_red_points.saturating_div(100);

            //update account
            account.redeemed_usdt = account.redeemed_usdt.saturating_add(redeemable_usdt);
            account.last_conversion = Some(self.env().block_timestamp());
            let usdt_transfer = self.send_usdt(caller, redeemable_usdt);
            if let Err(e) = usdt_transfer {
                return Err(e);
            }

            self.accounts.insert(caller, &account);
            //attempt to pay ancestors
            if let Some(ancestors) = self.get_ancestors(caller) {
                let result = self.update_ancestors_coefficients(&ancestors);
                if result.is_err() {
                    return Err(Error::NoAccountFound);
                }
            }

            Ok(redeemable_usdt)
        }

        pub fn send_tokens_to_amm(&mut self) -> Result<(), Error> {
            let amount = self.env().transferred_value();
            let transfer_result = self.env().transfer(self.amm_contract, amount);
            if transfer_result.is_err() {
                return Err(Error::ErrorTransferringToMainContract);
            }
            Ok(())
        }

        pub fn send_usdt_to_amm(self, usdt_amount: Balance) -> Result<(), Error> {
            self.send_usdt(self.amm_contract, usdt_amount)
        }

        #[ink(message)]
        pub fn give_green_points_usdt(
            &mut self,
            consumer_id: AccountId,
            usdt_amount: Balance
        ) -> Result<GreenPointsResult, Error> {
            let merchant_id: AccountId = self.env().caller();
            let validate_merchant = self.validate_merchant(merchant_id);
            if let Err(e) = validate_merchant {
                return Err(e);
            }
            let check_usdt = self.validate_usdt_transfer(merchant_id, usdt_amount);
            if let Err(e) = check_usdt {
                return Err(e);
            }
            let receive_usdt_result = self.receive_usdt_from_user(merchant_id, usdt_amount);
            if let Err(e) = receive_usdt_result {
                return Err(e);
            }

            // calculate green points
            let consumer_green_points = self.calculate_green_points(usdt_amount);
            let sixteen_percent = Perbill::from_rational(16u32, 100u32);
            let merchant_green_points = sixteen_percent.mul_floor(usdt_amount);

            //update accounts
            self.add_green_points(consumer_id, consumer_green_points);
            self.add_green_points(merchant_id, merchant_green_points);

            //convert to d9
            let conversion_result = self.amm_get_d9(usdt_amount);
            if let Err(e) = conversion_result {
                return Err(e);
            }

            // sendf to mining pool
            let d9_amount = conversion_result.unwrap().1;
            let to_mining_pool_result = self.send_to_mining_pool(d9_amount);
            if let Err(e) = to_mining_pool_result {
                return Err(e);
            }

            //emit event
            self.env().emit_event(GreenPointsTransaction {
                merchant: GreenPointsCreated {
                    account_id: merchant_id,
                    green_points: merchant_green_points,
                },
                consumer: GreenPointsCreated {
                    account_id: consumer_id,
                    green_points: consumer_green_points,
                },
            });

            Ok(GreenPointsResult {
                merchant: merchant_green_points,
                consumer: consumer_green_points,
            })
        }

        #[ink(message, payable)]
        pub fn give_green_points_d9(
            &mut self,
            consumer_id: AccountId
        ) -> Result<GreenPointsResult, Error> {
            let merchant_id = self.env().caller();
            let validate_merchant = self.validate_merchant(merchant_id);
            if let Err(e) = validate_merchant {
                return Err(e);
            }
            let d9_amount = self.env().transferred_value();

            //convert to usdt
            let conversion_result = self.amm_get_usdt(d9_amount);
            if let Err(e) = conversion_result {
                return Err(e);
            }
            let usdt_amount = conversion_result.unwrap().1;
            self.give_green_points_usdt(consumer_id, usdt_amount)
        }

        #[ink(message, payable)]
        pub fn pay_merchant_usdt(
            &mut self,
            merchant_id: AccountId,
            usdt_amount: Balance
        ) -> Result<GreenPointsResult, Error> {
            let consumer_id = self.env().caller();
            let validate_merchant = self.validate_merchant(merchant_id);
            if let Err(e) = validate_merchant {
                return Err(e);
            }
            //check usdt transfer
            let validate_result = self.validate_usdt_transfer(consumer_id, usdt_amount);
            if let Err(e) = validate_result {
                return Err(e);
            }

            self.process_payment(consumer_id, merchant_id, usdt_amount)
        }

        /// a customer pays a merchant using d9
        #[ink(message, payable)]
        pub fn pay_merchant_d9(
            &mut self,
            merchant_id: AccountId
        ) -> Result<GreenPointsResult, Error> {
            let payer = self.env().caller();
            let d9_amount = self.env().transferred_value();
            // validate merchant account
            let validate_merchant = self.validate_merchant(merchant_id);
            if let Err(e) = validate_merchant {
                return Err(e);
            }

            //convert to usdt
            let conversion_result = self.amm_get_usdt(d9_amount);
            if conversion_result.is_err() {
                return Err(Error::AMMConversionFailed);
            }

            //process payments
            let usdt_amount = conversion_result.unwrap().1;
            self.process_payment(payer, merchant_id, usdt_amount)
        }

        fn process_payment(
            &mut self,
            consumer_id: AccountId,
            merchant_id: AccountId,
            usdt_amount: Balance
        ) -> Result<GreenPointsResult, Error> {
            //send usdt to merchant
            let eighty_four_percent = Perbill::from_rational(84u32, 100u32);
            let merchant_payment = eighty_four_percent.mul_floor(usdt_amount);
            let send_usdt_result = self.send_usdt(merchant_id, merchant_payment);
            if let Err(e) = send_usdt_result {
                return Err(e);
            }

            //process green points
            let usdt_to_green = usdt_amount.saturating_sub(merchant_payment);
            let green_points = self.calculate_green_points(usdt_to_green);

            //update accounts
            self.add_green_points(merchant_id, green_points);
            self.add_green_points(consumer_id, green_points);

            // convert usdt to d9
            let conversion_result = self.convert_to_d9(usdt_to_green);
            if let Err(e) = conversion_result {
                return Err(e);
            }
            let d9_amount = conversion_result.unwrap();

            //send to mining pool

            let mining_pool_transfer = self.send_to_mining_pool(d9_amount);
            if let Err(e) = mining_pool_transfer {
                return Err(e);
            }

            self.env().emit_event(GreenPointsTransaction {
                merchant: GreenPointsCreated {
                    account_id: merchant_id,
                    green_points,
                },
                consumer: GreenPointsCreated {
                    account_id: consumer_id,
                    green_points,
                },
            });

            Ok(GreenPointsResult {
                merchant: green_points,
                consumer: green_points,
            })
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
        pub fn get_account(&self, account_id: AccountId) -> Option<Account> {
            self.accounts.get(&account_id)
        }

        #[ink(message)]
        pub fn change_amm_contract(&mut self, new_amm_contract: AccountId) -> Result<(), Error> {
            self.only_admin()?;
            self.amm_contract = new_amm_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_mining_pool(&mut self, new_mining_pool: AccountId) -> Result<(), Error> {
            self.only_admin()?;
            self.mining_pool = new_mining_pool;
            Ok(())
        }

        fn validate_usdt_transfer(&self, account: AccountId, amount: Balance) -> Result<(), Error> {
            let check_balance_result = self.validate_usdt_balance(account, amount);
            if check_balance_result.is_err() {
                return Err(Error::UserUSDTBalanceInsufficient);
            }
            let check_allowance_result = self.validate_usdt_allowance(account, amount);
            if check_allowance_result.is_err() {
                return Err(Error::InsufficientAllowance);
            }
            Ok(())
        }

        fn validate_usdt_balance(
            &self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let usdt_balance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("PSP22::balance_of"))
                    ).push_arg(account_id)
                )
                .returns::<Balance>()
                .invoke();
            if usdt_balance < amount {
                return Err(Error::UserUSDTBalanceInsufficient);
            }
            Ok(())
        }

        pub fn validate_usdt_allowance(
            &self,
            owner: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let allowance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::allowance")))
                        .push_arg(owner)
                        .push_arg(self.env().account_id())
                )
                .returns::<Balance>()
                .invoke();
            if allowance < amount {
                return Err(Error::InsufficientAllowance);
            }
            Ok(())
        }

        /// make sure it is a valid merchant account and their subscription is not expired
        fn validate_merchant(&self, account_id: AccountId) -> Result<(), Error> {
            let merchant_expiry_option: Option<Timestamp> = self.merchant_expiry.get(&account_id);
            if merchant_expiry_option.is_none() {
                return Err(Error::NoMerchantAccountFound);
            }
            let merchant_expiry = merchant_expiry_option.unwrap();
            if merchant_expiry < self.env().block_timestamp() {
                return Err(Error::MerchantAccountExpired);
            }
            Ok(())
        }

        fn convert_to_d9(&self, amount: Balance) -> Result<Balance, Error> {
            let grant_allowance_result = self.grant_amm_allowance(amount);
            if grant_allowance_result.is_err() {
                return Err(Error::GrantingAllowanceFailed);
            }
            let conversion_result = self.amm_get_d9(amount);
            if conversion_result.is_err() {
                return Err(Error::AMMConversionFailed);
            }
            let (_, d9_amount) = conversion_result.unwrap();

            Ok(d9_amount)
        }

        fn send_usdt(&self, recipient: AccountId, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer")))
                        .push_arg(recipient)
                        .push_arg(amount)
                        .push_arg([0u8])
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        pub fn receive_usdt_from_user(
            &self,
            sender: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer_from")))
                        .push_arg(sender)
                        .push_arg(self.env().account_id())
                        .push_arg(amount)
                        .push_arg([0u8])
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        fn grant_amm_allowance(&self, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::approve")))
                        .push_arg(self.amm_contract)
                        .push_arg(amount)
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        ///convert received usdt to d9 which will go to mining pool
        fn amm_get_d9(&self, amount: Balance) -> Result<(Currency, Balance), Error> {
            build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("get_d9"))).push_arg(amount)
                )
                .returns::<Result<(Currency, Balance), Error>>()
                .invoke()
        }

        /// call amm contract to get usdt, which will go to merchant
        fn amm_get_usdt(&self, amount: Balance) -> Result<(Currency, Balance), Error> {
            build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .transferred_value(amount)
                .exec_input(ExecutionInput::new(Selector::new(selector_bytes!("get_usdt"))))
                .returns::<Result<(Currency, Balance), Error>>()
                .invoke()
        }

        /// function to restrict access to admin
        fn only_admin(&self) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != self.admin {
                return Err(Error::OnlyAdmin);
            }
            Ok(())
        }

        ///get green points from usdt amount
        fn calculate_green_points(&self, amount: Balance) -> Balance {
            amount.saturating_mul(100)
        }

        /// base rate calculation is based on time.acceleration is based on ancestors
        ///
        /// 1 red point = 1 green point
        fn calculate_red_points_from_time(
            &self,
            green_points: Balance,
            last_redeem_timestamp: Timestamp
        ) -> Balance {
            // rate green points => red points
            let transmutation_rate = Perbill::from_rational(1u32, 2000u32);

            let days_since_last_redeem = self
                .env()
                .block_timestamp()
                .saturating_sub(last_redeem_timestamp)
                .saturating_div(self.milliseconds_day) as Balance;

            let base_red_points = transmutation_rate
                .mul_floor(green_points)
                .saturating_mul(days_since_last_redeem);

            base_red_points
        }

        /// acceleration rate calculation is based on ancestors
        ///
        /// 10% for parent, 1% for each ancestor
        fn calculate_red_points_from_relationships(
            &self,
            green_points: Balance,
            referral_coefficients: (Balance, Balance)
        ) -> Balance {
            // let transmutation_rate = Perbill::from_rational(1u32, 2000u32);
            let transmutation_rate = Perbill::from_rational(1u32, 2000u32);

            let ten_percent = Perbill::from_rational(1u32, 10u32);
            let sons_factored_green_points = ten_percent
                .mul_floor(referral_coefficients.0)
                .saturating_mul(green_points);

            let one_percent = Perbill::from_rational(1u32, 100u32);
            let grandsons_factored_green_points = one_percent
                .mul_floor(referral_coefficients.1)
                .saturating_mul(green_points);

            let red_points_from_sons = transmutation_rate.mul_floor(sons_factored_green_points);
            let red_points_from_grandsons = transmutation_rate.mul_floor(
                grandsons_factored_green_points
            );

            let total_red_points = red_points_from_sons.saturating_add(red_points_from_grandsons);
            total_red_points
        }

        /// send some amount to the mining pool
        fn send_to_mining_pool(&self, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0) // replace with an appropriate gas limit
                .transferred_value(amount)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(ink::selector_bytes!("process_merchant_payment"))
                    )
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        pub fn get_ancestors(&self, account_id: AccountId) -> Option<Vec<AccountId>> {
            let result = self.env().extension().get_ancestors(account_id);
            match result {
                Ok(ancestors) => ancestors,
                Err(_) => None,
            }
        }

        fn add_green_points(&mut self, account_id: AccountId, amount: Balance) {
            let mut account = self.accounts
                .get(&account_id)
                .unwrap_or(Account::new(self.env().block_timestamp()));
            account.green_points = account.green_points.saturating_add(amount);
            self.accounts.insert(account_id, &account);
        }

        fn update_ancestors_coefficients(&mut self, ancestors: &[AccountId]) -> Result<(), Error> {
            //modify parent
            let parent = ancestors.first().unwrap();
            if let Some(mut account) = self.accounts.get(parent) {
                account.relationship_factors.0 += 1;
                account.relationship_factors = account.relationship_factors;
                self.accounts.insert(parent, &account);
            }

            //modify others
            for ancestor in ancestors.iter().skip(1) {
                if let Some(mut account) = self.accounts.get(ancestor) {
                    account.relationship_factors.1 += 1;
                    account.relationship_factors = account.relationship_factors;
                    self.accounts.insert(ancestor, &account);
                }
            }
            Ok(())
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
            ink::env::test::set_caller::<ink::env::DefaultEnvironment>(caller);
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
            let contract = D9MerchantMining::new(
                subscription_fee,
                default_accounts.bob,
                default_accounts.charlie
            );
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

        fn get_expiry() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);

            //create subscription
            let payment_amount: Balance = 1000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let subscription_result = contract.subscribe();
            assert!(subscription_result.is_ok());
            //one month
            assert_eq!(subscription_result.unwrap(), 2592000000);

            //check account
            let expiry = contract.get_expiry(default_accounts.alice);
            assert_eq!(expiry, Ok(2592000000));
        }

        #[ink::test]
        fn validate_merchant() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);

            contract.merchant.expiry.insert(default_accounts.alice, 1000);
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

            // let merchant_expiry = contract.get_expiryOk()(new_merchant);z
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

            let red_points = contract.calculate_red_points_from_time(
                200_000_000,
                last_redeem_timestamp
            );

            assert_eq!(red_points, 1_000_000)
        }

        #[ink::test]
        fn pay_for_subscription() {
            let (default_accounts, mut contract) = default_setup();
            init_calling_env(default_accounts.alice);
            //create subscription
            let payment_amount: Balance = 10_000_000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let subscription_result = contract.subscribe();
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
            let _ = contract.subscribe();

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
            let _ = contract.subscribe();

            //give green points
            let payment_amount: Balance = 10000;
            set_value_transferred::<DefaultEnvironment>(payment_amount);
            let _ = contract.give_green_points(default_accounts.bob);
            move_time_forward(86_400_000 * 100);

            //redeem d9
            set_caller::<DefaultEnvironment>(default_accounts.bob);
            let redemption_result = contract.redeem_usdt();
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
