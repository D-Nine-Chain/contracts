#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod d9_merchant_mining {
    use super::*;
    use ink::env::call::{build_call, ExecutionInput, Selector};
    use ink::env::Error as EnvError;
    use ink::prelude::vec::Vec;
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{Decode, Encode};
    use sp_arithmetic::Perbill;

    #[ink(storage)]
    pub struct D9MerchantMining {
        /// accountId to merchant account expiry date
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

    #[derive(Decode, Encode, Clone)]
    #[cfg_attr(
        feature = "std",
        derive(
            Debug,
            PartialEq,
            Eq,
            ink::storage::traits::StorageLayout,
            scale_info::TypeInfo
        )
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
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Currency {
        D9,
        USDT,
    }
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Direction(Currency, Currency);
    // data to return to user
    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(
            Debug,
            PartialEq,
            Eq,
            ink::storage::traits::StorageLayout,
            scale_info::TypeInfo
        )
    )]
    pub struct GreenPointsResult {
        merchant: Balance,
        consumer: Balance,
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
        TransferringToMainContract,
        TransferringToUSDTToMerchant,
        UserUSDTBalanceInsufficient,
        D9TransferFailed,
        USDTTransferFailed,
        OnlyAdmin,
        GrantingAllowanceFailed,
        AMMConversionFailed,
        ReceivingUSDTFromUser,
        ConvertingToD9,
        SendUSDTToMerchant,
        SendingD9ToMiningPool,
        SendingUSDTToAMM,
        GettingUSDTFromAMM,
        RedeemD9TransferFailed,
        SomeEnvironmentError,
        CalledContractTrapped,
        CalledContractReverted,
        NotCallable,
        SomeDecodeError,
        SomeOffChainError,
        CalleeTrapped,
        CalleeReverted,
        KeyNotFound,
        _BelowSubsistenceThreshold,
        TransferFailed,
        _EndowmentTooLow,
        CodeNotFound,
        Unknown,
        LoggingDisabled,
        CallRuntimeFailed,
        EcdsaRecoveryFailed,
        ErrorGettingEstimate,
        CrossContractCallErrorGettingEstimate,
        NoAccountCantCreateMerchantAccount,
        PointsInsufficientToCreateMerchantAccount,
    }

    impl From<EnvError> for Error {
        fn from(error: EnvError) -> Self {
            match error {
                EnvError::CalleeTrapped => Self::CalledContractTrapped,
                EnvError::CalleeReverted => Self::CalledContractReverted,
                EnvError::NotCallable => Self::NotCallable,
                EnvError::KeyNotFound => Self::KeyNotFound,
                EnvError::_BelowSubsistenceThreshold => Self::_BelowSubsistenceThreshold,
                EnvError::TransferFailed => Self::TransferFailed,
                EnvError::_EndowmentTooLow => Self::_EndowmentTooLow,
                EnvError::CodeNotFound => Self::CodeNotFound,
                EnvError::Unknown => Self::Unknown,
                EnvError::LoggingDisabled => Self::LoggingDisabled,
                EnvError::CallRuntimeFailed => Self::CallRuntimeFailed,
                EnvError::EcdsaRecoveryFailed => Self::EcdsaRecoveryFailed,
                _ => Self::SomeEnvironmentError,
            }
        }
    }

    #[ink(event)]
    pub struct SubscriptionExtended {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        usdt: Balance,
        #[ink(topic)]
        expiry: Timestamp,
    }

    #[ink(event)]
    pub struct D9Redeemed {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        redeemed_d9: Balance,
    }

    // event for creation of green points
    #[ink(event)]
    pub struct GreenPointsTransaction {
        #[ink(topic)]
        merchant: GreenPointsCreated,
        #[ink(topic)]
        consumer: GreenPointsCreated,
    }

    #[ink(event)]
    pub struct D9MerchantPaymentSent {
        #[ink(topic)]
        merchant: AccountId,
        #[ink(topic)]
        consumer: AccountId,
        #[ink(topic)]
        amount: Balance,
    }
    #[ink(event)]
    pub struct USDTMerchantPaymentSent {
        #[ink(topic)]
        merchant: AccountId,
        #[ink(topic)]
        consumer: AccountId,
        #[ink(topic)]
        amount: Balance,
    }

    #[ink(event)]
    pub struct GivePointsUSDT {
        #[ink(topic)]
        consumer: AccountId,
        #[ink(topic)]
        merchant: AccountId,
        amount: Balance,
    }

    // a struct associated with the GreenPointsTransaction event
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct GreenPointsCreated {
        account_id: AccountId,
        green_points: Balance,
    }

    impl D9MerchantMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(
            amm_contract: AccountId,
            mining_pool: AccountId,
            usdt_contract: AccountId,
        ) -> Self {
            Self {
                admin: Self::env().caller(),
                amm_contract,
                usdt_contract,
                mining_pool,
                merchant_expiry: Default::default(),
                accounts: Default::default(),
                subscription_fee: 1000,
                milliseconds_day: 86_400_000,
            }
        }

        // old main xssaidD9aqTCqsbLn1ncF2gtZyr4MreBXzXT8fquLZfcMrB
        /// create merchant account subscription
        #[ink(message)]
        pub fn subscribe(&mut self, usdt_amount: Balance) -> Result<Timestamp, Error> {
            let merchant_id = self.env().caller();
            if usdt_amount < self.subscription_fee {
                return Err(Error::InsufficientPayment);
            }
            self.check_subscription_permissibility(merchant_id)?;
            self.validate_usdt_transfer(merchant_id, usdt_amount)?;
            self.receive_usdt_from_user(merchant_id, usdt_amount)?;
            let send_usdt_result = self.contract_sends_usdt_to(self.amm_contract, usdt_amount);
            if send_usdt_result.is_err() {
                return Err(Error::SendingUSDTToAMM);
            }

            let update_expiry_result = self.update_subscription(merchant_id, usdt_amount);

            update_expiry_result
        }

        ///create/update subscription, returns new expiry `Timestamp` Result
        fn update_subscription(
            &mut self,
            account_id: AccountId,
            amount: Balance,
        ) -> Result<Timestamp, Error> {
            let months = amount.saturating_div(self.subscription_fee) as Timestamp;
            if months == 0 {
                return Err(Error::InsufficientPayment);
            }
            let one_month: Timestamp = self.milliseconds_day * 30;
            let current_expiry: Timestamp = match self.merchant_expiry.get(&account_id) {
                Some(expiry) => {
                    if expiry < self.env().block_timestamp() {
                        self.env().block_timestamp()
                    } else {
                        expiry
                    }
                }
                None => self.env().block_timestamp(),
            };
            let new_expiry = current_expiry.saturating_add(months.saturating_mul(one_month));
            self.merchant_expiry.insert(account_id.clone(), &new_expiry);
            self.env().emit_event(SubscriptionExtended {
                account_id,
                usdt: amount,
                expiry: new_expiry,
            });
            Ok(new_expiry)
        }
        // In merchant mining contract, add a new redemption function:

        #[ink(message)]
        pub fn redeem_d9_with_price_protection(
            &mut self,
            price_oracle: AccountId,
        ) -> Result<Balance, Error> {
            // Get account (same as regular redeem_d9)
            let caller = self.env().caller();
            let maybe_account = self.accounts.get(&caller);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let mut account = maybe_account.unwrap();

            // Calculate redeemable points (same logic)
            if account.green_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            let redeemable_red_points = self.calc_total_redeemable_red_points(&account);
            if redeemable_red_points == 0 {
                return Err(Error::NothingToRedeem);
            }

            // Check 24hr lockout (same logic)
            let is_within_24_hr_lockout = match account.last_conversion {
                Some(last_conversion) => {
                    let twenty_four_hours_prior =
                        self.env().block_timestamp().saturating_sub(86_400_000);
                    twenty_four_hours_prior < last_conversion
                }
                None => false,
            };
            if is_within_24_hr_lockout {
                return Err(Error::NothingToRedeem);
            }

            // Call disburse with oracle
            let disburse_result = self.disburse_d9_with_oracle(
                caller,
                &mut account,
                redeemable_red_points,
                price_oracle,
            );
            self.accounts.insert(caller, &account);
            return disburse_result;
        }

        // New disburse function that uses oracle
        fn disburse_d9_with_oracle(
            &mut self,
            recipient_id: AccountId,
            account: &mut Account,
            redeemable_red_points: Balance,
            price_oracle: AccountId,
        ) -> Result<Balance, Error> {
            let redeemable_usdt = redeemable_red_points.saturating_div(100);

            // Call mining pool with oracle
            let d9_amount =
                self.mining_pool_redeem_with_oracle(recipient_id, redeemable_usdt, price_oracle)?;

            // Rest is same as original disburse_d9
            account.redeemed_d9 = account.redeemed_d9.saturating_add(d9_amount);
            account.relationship_factors = (0, 0);

            // Process ancestors (same as before)
            let last_redeem_timestamp = account.last_conversion.unwrap_or(account.created_at);
            let time_based_red_points =
                self.calc_red_points_from_time(account.green_points, last_redeem_timestamp);
            if let Some(ancestors) = self.get_ancestors(recipient_id) {
                let _ = self.update_ancestors_coefficients(&ancestors, time_based_red_points);
            }

            account.last_conversion = Some(self.env().block_timestamp());
            account.green_points = account.green_points.saturating_sub(redeemable_red_points);

            self.env().emit_event(D9Redeemed {
                account_id: recipient_id,
                redeemed_d9: d9_amount,
            });

            Ok(d9_amount)
        }

        // Helper function to call mining pool with oracle
        fn mining_pool_redeem_with_oracle(
            &self,
            user_account: AccountId,
            redeemable_usdt: Balance,
            price_oracle: AccountId,
        ) -> Result<Balance, Error> {
            let result = build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!(
                        "merchant_user_redeem_d9_with_oracle"
                    )))
                    .push_arg(user_account)
                    .push_arg(redeemable_usdt)
                    .push_arg(price_oracle),
                )
                .returns::<Result<Balance, Error>>()
                .try_invoke()?;
            result.unwrap()
        }

        // Keep original redeem_d9() function unchanged for backward compatibility
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
            if account.green_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            let redeemable_red_points = self.calc_total_redeemable_red_points(&account);
            if redeemable_red_points == 0 {
                return Err(Error::NothingToRedeem);
            }
            let is_within_24_hr_lockout = match account.last_conversion {
                Some(last_conversion) => {
                    let twenty_four_hours_prior =
                        self.env().block_timestamp().saturating_sub(86_400_000);
                    twenty_four_hours_prior < last_conversion
                }
                None => false,
            };
            if is_within_24_hr_lockout {
                return Err(Error::NothingToRedeem);
            }
            let disburse_result = self.disburse_d9(caller, &mut account, redeemable_red_points);
            self.accounts.insert(caller, &account);
            return disburse_result;
        }

        /// total redeemable red points will never be more than account's remaining green points
        fn calc_total_redeemable_red_points(&self, account: &Account) -> Balance {
            let last_redeem_timestamp = account.last_conversion.unwrap_or(account.created_at);
            let time_based_red_points =
                self.calc_red_points_from_time(account.green_points, last_redeem_timestamp);
            let relationship_based_red_points =
                self.calc_red_points_from_relationships(account.relationship_factors);
            let total_red_points =
                time_based_red_points.saturating_add(relationship_based_red_points);
            let redeemable_red_points = {
                if total_red_points > account.green_points {
                    account.green_points
                } else {
                    total_red_points
                }
            };
            redeemable_red_points
        }

        fn disburse_d9(
            &mut self,
            recipient_id: AccountId,
            account: &mut Account,
            redeemable_red_points: Balance,
        ) -> Result<Balance, Error> {
            //calculated red points => d9 conversion
            let redeemable_usdt = redeemable_red_points.saturating_div(100);
            let redeem_result = self.mining_pool_redeem(recipient_id, redeemable_usdt);
            if redeem_result.is_err() {
                return Err(Error::RedeemD9TransferFailed);
            }
            let d9_amount = redeem_result.unwrap();
            //update account
            account.redeemed_d9 = account.redeemed_d9.saturating_add(d9_amount);

            account.relationship_factors = (0, 0);

            //attempt to pay ancestors
            //calculate green => red points conversion
            let last_redeem_timestamp = account.last_conversion.unwrap_or(account.created_at);
            let time_based_red_points =
                self.calc_red_points_from_time(account.green_points, last_redeem_timestamp);
            if let Some(ancestors) = self.get_ancestors(recipient_id) {
                let _ = self.update_ancestors_coefficients(&ancestors, time_based_red_points);
            }

            account.last_conversion = Some(self.env().block_timestamp());
            account.green_points = account.green_points.saturating_sub(redeemable_red_points);

            self.env().emit_event(D9Redeemed {
                account_id: recipient_id,
                redeemed_d9: d9_amount,
            });

            Ok(d9_amount)
        }

        fn mining_pool_redeem(
            &self,
            user_account: AccountId,
            redeemable_usdt: Balance,
        ) -> Result<Balance, Error> {
            let result = build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("merchant_user_redeem_d9")))
                        .push_arg(user_account)
                        .push_arg(redeemable_usdt),
                )
                .returns::<Result<Balance, Error>>()
                .try_invoke()?;
            result.unwrap()
        }

        #[ink(message, payable)]
        pub fn give_green_points_d9(
            &mut self,
            consumer_id: AccountId,
        ) -> Result<GreenPointsResult, Error> {
            let merchant_id = self.env().caller();
            self.validate_merchant(merchant_id)?;
            let d9_amount = self.env().transferred_value();
            let usdt_amount = self.estimate_usdt(d9_amount)?;
            // Convert to USDT and delegate to give_green_points_internal
            let green_points_result_result =
                self.give_green_points_internal(consumer_id, usdt_amount);
            if let Err(e) = green_points_result_result {
                return Err(e);
            }
            self.call_mining_pool_to_process(merchant_id, d9_amount)?;
            green_points_result_result
        }

        #[ink(message)]
        pub fn give_green_points_usdt(
            &mut self,
            consumer_id: AccountId,
            usdt_payment: Balance,
        ) -> Result<GreenPointsResult, Error> {
            let merchant_id = self.env().caller();
            self.validate_merchant(merchant_id)?;
            self.validate_usdt_transfer(merchant_id, usdt_payment)?;
            self.receive_usdt_from_user(merchant_id, usdt_payment)?;

            // Delegate to give_green_points_internal
            let green_points_result_result =
                self.give_green_points_internal(consumer_id, usdt_payment);
            if let Err(e) = green_points_result_result {
                self.contract_sends_usdt_to(merchant_id, usdt_payment)?;
                return Err(e);
            }
            let d9_amount = self.convert_to_d9(usdt_payment)?;
            self.call_mining_pool_to_process(merchant_id, d9_amount)?;
            self.env().emit_event(GivePointsUSDT {
                consumer: consumer_id,
                merchant: merchant_id,
                amount: usdt_payment,
            });
            Ok(green_points_result_result.unwrap())
        }

        fn give_green_points_internal(
            &mut self,
            consumer_id: AccountId,
            amount: Balance,
        ) -> Result<GreenPointsResult, Error> {
            // Calculate green points
            let usdt_amount_to_green = amount.saturating_mul(100).saturating_div(16);
            let consumer_green_points = self.calculate_green_points(usdt_amount_to_green);
            let merchant_green_points =
                Perbill::from_rational(16u32, 100u32).mul_floor(consumer_green_points);

            // Update accounts
            let add_consumer_points_result =
                self.add_green_points(consumer_id, consumer_green_points, true);
            if let Err(e) = add_consumer_points_result {
                return Err(e);
            }
            let add_merchant_points_result =
                self.add_green_points(self.env().caller(), merchant_green_points, false);
            if let Err(e) = add_merchant_points_result {
                return Err(e);
            }
            // Emit event
            self.env().emit_event(GreenPointsTransaction {
                merchant: GreenPointsCreated {
                    account_id: self.env().caller(),
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
        pub fn send_usdt_payment_to_merchant(
            &mut self,
            merchant_id: AccountId,
            usdt_amount: Balance,
        ) -> Result<GreenPointsResult, Error> {
            let consumer_id = self.env().caller();
            self.validate_merchant(merchant_id)?;
            self.validate_usdt_transfer(consumer_id, usdt_amount)?;
            self.receive_usdt_from_user(consumer_id, usdt_amount)?;
            self.env().emit_event(USDTMerchantPaymentSent {
                merchant: merchant_id,
                consumer: consumer_id,
                amount: usdt_amount,
            });
            self.finish_processing_payment(consumer_id, merchant_id, usdt_amount)
        }

        /// a customer pays a merchant using d9
        #[ink(message, payable)]
        pub fn send_d9_payment_to_merchant(
            &mut self,
            merchant_id: AccountId,
        ) -> Result<GreenPointsResult, Error> {
            let payer = self.env().caller();
            let d9_amount = self.env().transferred_value();
            // validate merchant account
            let validate_merchant = self.validate_merchant(merchant_id);
            if let Err(e) = validate_merchant {
                return Err(e);
            }

            //convert to usdt
            let conversion_result = self.convert_to_usdt(d9_amount);
            if conversion_result.is_err() {
                return Err(Error::AMMConversionFailed);
            }

            //process payments
            let usdt_amount = conversion_result.unwrap();

            self.env().emit_event(D9MerchantPaymentSent {
                merchant: merchant_id,
                consumer: payer,
                amount: d9_amount,
            });
            self.finish_processing_payment(payer, merchant_id, usdt_amount)
        }

        fn finish_processing_payment(
            &mut self,
            consumer_id: AccountId,
            merchant_id: AccountId,
            usdt_amount: Balance,
        ) -> Result<GreenPointsResult, Error> {
            //send usdt to merchant
            let eighty_four_percent = Perbill::from_rational(84u32, 100u32);
            let merchant_payment = eighty_four_percent.mul_floor(usdt_amount);

            let send_usdt_result = self.contract_sends_usdt_to(merchant_id, merchant_payment);
            if send_usdt_result.is_err() {
                return Err(Error::SendUSDTToMerchant);
            }

            //process green points
            let merchant_usdt_to_green = usdt_amount.saturating_sub(merchant_payment);
            let merchant_green_points = self.calculate_green_points(merchant_usdt_to_green);
            let consumer_green_points = self.calculate_green_points(usdt_amount);
            //update accounts
            let add_merchant_points_result =
                self.add_green_points(merchant_id, merchant_green_points, false);
            if let Err(e) = add_merchant_points_result {
                return Err(e);
            }
            let add_consumer_points_result =
                self.add_green_points(consumer_id, consumer_green_points, true);
            if let Err(e) = add_consumer_points_result {
                return Err(e);
            }

            // convert usdt to d9
            let conversion_result = self.convert_to_d9(merchant_usdt_to_green);
            if let Err(e) = conversion_result {
                return Err(e);
            }
            let d9_amount = conversion_result.unwrap();

            //send to mining pool
            self.call_mining_pool_to_process(merchant_id, d9_amount)?;

            // self.credit_pool(d9_amount);
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

        #[ink(message)]
        pub fn get_mining_pool(&self) -> AccountId {
            self.mining_pool
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

        fn check_subscription_permissibility(&self, account_id: AccountId) -> Result<(), Error> {
            let account_option = self.accounts.get(&account_id);
            if account_option.is_none() {
                return Err(Error::NoAccountCantCreateMerchantAccount);
            }
            let account = account_option.unwrap();
            let threshold_points: Balance = 500_000_000;
            if account.green_points < threshold_points {
                return Err(Error::PointsInsufficientToCreateMerchantAccount);
            }
            Ok(())
        }

        fn validate_usdt_transfer(&self, account: AccountId, amount: Balance) -> Result<(), Error> {
            let check_balance_result = self.validate_usdt_balance(account, amount);
            if check_balance_result.is_err() {
                return Err(Error::UserUSDTBalanceInsufficient);
            }
            let check_allowance_result = self.validate_usdt_allowance(account, amount);
            if let Err(e) = check_allowance_result {
                return Err(e);
            }
            Ok(())
        }

        fn validate_usdt_balance(
            &self,
            account_id: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            let usdt_balance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::balance_of")))
                        .push_arg(account_id),
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
            amount: Balance,
        ) -> Result<(), Error> {
            let allowance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::allowance")))
                        .push_arg(owner)
                        .push_arg(self.env().account_id()),
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

        fn convert_to_d9(&mut self, amount: Balance) -> Result<Balance, Error> {
            let grant_allowance_result = self.grant_amm_allowance(amount);
            if grant_allowance_result.is_err() {
                return Err(Error::GrantingAllowanceFailed);
            }
            let d9_amount = self.amm_get_d9(amount)?;

            Ok(d9_amount)
        }

        fn contract_sends_usdt_to(
            &self,
            recipient: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer")))
                        .push_arg(recipient)
                        .push_arg(amount)
                        .push_arg([0u8]),
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        pub fn receive_usdt_from_user(
            &self,
            sender: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer_from")))
                        .push_arg(sender)
                        .push_arg(self.env().account_id())
                        .push_arg(amount)
                        .push_arg([0u8]),
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }
        //xjyLYnZBRhYYjUKjCp8UiHnmcjHmkPfRSBxTiLLMoEwtzwp
        //d40a697875ef7a24aaed19ab41e1395675a1d84a5ddbc78a5a342e87c2d580f6
        //89151c651f568f7ae1f1156c3409d329bd5ccfc0eb9fc29b38b25d8b8bf831fe <- factor fix

        fn grant_amm_allowance(&mut self, amount: Balance) -> Result<(), Error> {
            let call_result = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::approve")))
                        .push_arg(self.amm_contract)
                        .push_arg(amount),
                )
                .returns::<Result<(), Error>>()
                .try_invoke()?;
            call_result.unwrap()
        }

        ///convert received usdt to d9 which will go to mining pool
        fn amm_get_d9(&self, amount: Balance) -> Result<Balance, Error> {
            let call_result = build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("get_d9"))).push_arg(amount),
                )
                .returns::<Result<Balance, Error>>()
                .try_invoke()?;
            call_result.unwrap()
        }

        /// call amm contract to get usdt, which will go to merchant

        fn convert_to_usdt(&self, amount: Balance) -> Result<Balance, Error> {
            let result = build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .transferred_value(amount)
                .exec_input(ExecutionInput::new(Selector::new(selector_bytes!(
                    "get_usdt"
                ))))
                .returns::<Result<Balance, Error>>()
                .try_invoke()?;
            result.unwrap()
        }

        fn estimate_usdt(&self, amount: Balance) -> Result<Balance, Error> {
            let direction = Direction(Currency::D9, Currency::USDT);
            // this result is to catch any error in calling originating from the environment
            let cross_contract_call_result = build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("estimate_exchange")))
                        .push_arg(direction)
                        .push_arg(amount),
                )
                .returns::<Result<(Balance, Balance), Error>>()
                .try_invoke();
            // this result will return the value or some error from the contract itself
            if cross_contract_call_result.is_err() {
                return Err(Error::CrossContractCallErrorGettingEstimate);
            }
            let method_call_result = cross_contract_call_result.unwrap();
            if method_call_result.is_err() {
                return Err(Error::ErrorGettingEstimate);
            }
            let something = method_call_result.unwrap();
            if something.is_err() {
                return Err(Error::ErrorGettingEstimate);
            }
            let usdt_balance = something.unwrap().1;
            Ok(usdt_balance)
        }

        /// function to restrict access to admin
        fn only_admin(&self) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != self.admin {
                return Err(Error::OnlyAdmin);
            }
            Ok(())
        }

        #[ink(message)]
        pub fn change_admin(&mut self, new_admin: AccountId) -> Result<(), Error> {
            self.only_admin()?;
            self.admin = new_admin;
            Ok(())
        }

        ///get green points from usdt amount
        fn calculate_green_points(&self, amount: Balance) -> Balance {
            amount.saturating_mul(100)
        }

        /// base rate calculation is based on time.acceleration is based on ancestors
        ///
        /// 1 red point = 1 green point
        fn calc_red_points_from_time(
            &self,
            green_points: Balance,
            last_redeem_timestamp: Timestamp,
        ) -> Balance {
            // rate green points => red points
            let transmutation_rate = Perbill::from_rational(1u32, 2000u32);

            let days_since_last_redeem =
                self.env()
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
        fn calc_red_points_from_relationships(
            &self,
            // red_points: Balance,
            referral_coefficients: (Balance, Balance),
        ) -> Balance {
            let total_red_points = referral_coefficients
                .0
                .saturating_add(referral_coefficients.1);
            total_red_points
        }

        /// send some amount to the mining pool
        fn call_mining_pool_to_process(
            &self,
            merchant_id: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            let _ = build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0) // replace with an appropriate gas limit
                .transferred_value(amount)
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!(
                        "process_merchant_payment"
                    )))
                    .push_arg(merchant_id),
                )
                .returns::<Result<(), Error>>()
                .try_invoke()?;
            Ok(())
        }

        pub fn get_ancestors(&self, account_id: AccountId) -> Option<Vec<AccountId>> {
            let result = self.env().extension().get_ancestors(account_id);
            match result {
                Ok(ancestors) => ancestors,
                Err(_) => None,
            }
        }

        fn add_green_points(
            &mut self,
            account_id: AccountId,
            amount: Balance,
            is_consumer: bool,
        ) -> Result<(), Error> {
            let mut account = self
                .accounts
                .get(&account_id)
                .unwrap_or(Account::new(self.env().block_timestamp()));
            let redeemable_red_points = self.calc_total_redeemable_red_points(&account);
            let twenty_four_hours_prior = self.env().block_timestamp().saturating_sub(86_400_000);
            let permit_based_on_last_conversion: bool = match account.last_conversion {
                Some(last_conversion) => last_conversion < twenty_four_hours_prior,
                None => true,
            };

            if redeemable_red_points > 0 && permit_based_on_last_conversion && is_consumer {
                let disburse_result =
                    self.disburse_d9(account_id, &mut account, redeemable_red_points);
                if let Err(e) = disburse_result {
                    return Err(e);
                }
            }
            account.green_points = account.green_points.saturating_add(amount);
            self.accounts.insert(account_id, &account);
            Ok(())
        }

        /// update referral coefficients for predecessor accounts
        fn update_ancestors_coefficients(
            &mut self,
            ancestors: &[AccountId],
            withdraw_amount: Balance,
        ) {
            //modify parent
            let parent = ancestors.first();
            if let Some(parent) = parent {
                if let Some(mut account) = self.accounts.get(parent) {
                    let ten_percent = Perbill::from_rational(1u32, 10u32);
                    let parent_bonus = ten_percent.mul_floor(withdraw_amount);
                    account.relationship_factors.0 =
                        account.relationship_factors.0.saturating_add(parent_bonus);
                    account.relationship_factors = account.relationship_factors;
                    self.accounts.insert(parent, &account);
                }
            }

            //modify others
            for ancestor in ancestors.iter().skip(1) {
                if let Some(mut account) = self.accounts.get(ancestor) {
                    let one_percent = Perbill::from_rational(1u32, 100u32);
                    let ancestor_bonus: Balance = one_percent.mul_floor(withdraw_amount);
                    account.relationship_factors.1 = account
                        .relationship_factors
                        .1
                        .saturating_add(ancestor_bonus);
                    account.relationship_factors = account.relationship_factors;
                    self.accounts.insert(ancestor, &account);
                }
            }
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        use core::default;

        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;
        use ink::env::test::{set_caller, set_value_transferred};
        use ink::env::DefaultEnvironment;
        static ONE_MONTH_MILLISECONDS: Timestamp = 86_400_000 * 30;

        /// prepare default accounts and contract address for tests
        fn init_accounts() -> ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> {
            let default_accounts: ink::env::test::DefaultAccounts<ink::env::DefaultEnvironment> =
                ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
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
            let contract = D9MerchantMining::new(
                default_accounts.alice,
                default_accounts.bob,
                default_accounts.charlie,
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
                current_block_time + move_forward_by,
            );
            let _ = ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
        }

        #[ink::test]
        fn redeem_d9() {
            let (default_accounts, mut contract) = default_setup();

            //prep contract calling env
            init_calling_env(default_accounts.alice);
            let account: Account = Account {
                green_points: 200000000,
                relationship_factors: (0, 0),
                last_conversion: None,
                redeemed_usdt: 0,
                redeemed_d9: 0,
                created_at: 0,
            };
            set_block_time(0);
            contract.accounts.insert(default_accounts.alice, &account);
            move_time_forward(100_000_000);

            //redeem d9
            set_caller::<DefaultEnvironment>(default_accounts.alice);
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
        use ink_e2e::{account_id, build_message, AccountKeyring};
        use mining_pool::mining_pool::MiningPool;
        use mining_pool::mining_pool::MiningPoolRef;
        /// The End-to-End test `Result` type.
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
        /// We test that we can upload and instantiate the contract using its default constructor.
        #[ink_e2e::test]
        async fn mining_pool_processing_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // mining pool construction
            let constructor = D9MerchantMiningRef::new(
                client.alice().account_id,
                client.bob().account_id,
                client.charlie().account_id,
            );

            Ok(())
        }
    }
}
