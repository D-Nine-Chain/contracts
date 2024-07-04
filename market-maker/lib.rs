#![cfg_attr(not(feature = "std"), no_std, no_main)]
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod market_maker {
    use super::*;
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use ink::{
        env::{
            call::{build_call, ExecutionInput, Selector},
            Error as EnvError,
        },
        LangError,
    };
    use scale::{Decode, Encode};
    use sp_arithmetic::Perquintill;
    use substrate_fixed::{types::extra::U28, FixedU128};
    type FixedBalance = FixedU128<U28>;
    #[ink(storage)]
    pub struct MarketMaker {
        /// contract for usdt coin
        usdt_contract: AccountId,
        /// part per quintillion
        fee_percent: u32,
        /// total fees collected
        fee_total: Balance,
        ///represents numerator of a percent
        liquidity_tolerance_percent: u32,
        /// providers of contract liquidity
        liquidity_providers: Mapping<AccountId, Balance>,
        /// total number of liquidity pool tokens
        total_lp_tokens: Balance,
        admin: AccountId,
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

    #[ink(event)]
    pub struct LiquidityAdded {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        usdt: Balance,
        #[ink(topic)]
        d9: Balance,
    }

    #[ink(event)]
    pub struct LiquidityRemoved {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        usdt: Balance,
        #[ink(topic)]
        d9: Balance,
    }

    #[ink(event)]
    pub struct D9ToUSDTConversion {
        #[ink(topic)]
        account_id: AccountId,
        #[ink(topic)]
        usdt: Balance,
        #[ink(topic)]
        d9: Balance,
    }

    #[ink(event)]
    pub struct USDTToD9Conversion {
        #[ink(topic)]
        account_id: AccountId,
        usdt: Balance,
        d9: Balance,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum ContractError {
        D9orUSDTProvidedLiquidityAtZero,
        ConversionAmountTooLow,
        CouldntTransferUSDTFromUser,
        InsufficientLiquidity(Currency),
        USDTAllowanceInsufficient,
        MarketMakerHasInsufficientFunds(Currency),
        InsufficientLiquidityProvided,
        USDTBalanceInsufficient,
        LiquidityProviderNotFound,
        LiquidityAddedBeyondTolerance(Balance, Balance),
        InsufficientLPTokens,
        InsufficientContractLPTokens,
        DivisionByZero,
        MultiplicationError,
        USDTTooSmall,
        USDTTooMuch,
        LiquidityTooLow,
        ScaleDecodeError,
        CalleeTrapped,
        CalleeReverted,
        StorageKeyNotFound,
        TransferFailed,
        ExplicitlyUnknownError,
        OtherEnvironmentError,
        LangError(LangError),
    }
    impl From<EnvError> for ContractError {
        fn from(_error: EnvError) -> Self {
            match _error {
                EnvError::Decode(_e) => ContractError::ScaleDecodeError,
                EnvError::CalleeTrapped => ContractError::CalleeTrapped,
                EnvError::CalleeReverted => ContractError::CalleeReverted,
                EnvError::KeyNotFound => ContractError::StorageKeyNotFound,
                EnvError::TransferFailed => ContractError::TransferFailed,
                _ => ContractError::OtherEnvironmentError,
            }
        }
    }

    impl MarketMaker {
        #[ink(constructor)]
        pub fn new(
            usdt_contract: AccountId,
            fee_percent: u32,
            liquidity_tolerance_percent: u32,
        ) -> Self {
            assert!(
                liquidity_tolerance_percent <= 100,
                "tolerance must be 0 <= x <= 100"
            );
            Self {
                admin: Self::env().caller(),
                usdt_contract,
                fee_percent,
                fee_total: Default::default(),
                liquidity_tolerance_percent,
                liquidity_providers: Default::default(),
                total_lp_tokens: Default::default(),
            }
        }

        #[ink(message)]
        pub fn change_admin(&mut self, new_admin: AccountId) {
            self.only_admin();
            self.admin = new_admin;
        }

        pub fn change_fee_percent(&mut self, new_fee_percent: u32) -> () {
            self.only_admin();
            self.fee_percent = new_fee_percent;
        }

        /// get pool balances (d9, usdt)
        #[ink(message)]
        pub fn get_currency_reserves(&self) -> Result<(Balance, Balance), ContractError> {
            let d9_balance: Balance = self.env().balance();
            let usdt_balance: Balance = self.get_usdt_balance(self.env().account_id())?;
            Ok((d9_balance, usdt_balance))
        }

        #[ink(message)]
        pub fn get_liquidity_provider(&self, account_id: AccountId) -> Option<Balance> {
            self.liquidity_providers.get(&account_id)
        }
        /// add liquidity by adding tokens to the reserves
        #[ink(message, payable)]
        pub fn add_liquidity(&mut self, usdt_liquidity: Balance) -> Result<(), ContractError> {
            let caller = self.env().caller();
            // greater than zero checks
            let d9_liquidity = self.env().transferred_value();
            if usdt_liquidity == 0 || d9_liquidity == 0 {
                return Err(ContractError::D9orUSDTProvidedLiquidityAtZero);
            }
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves()?;
            if usdt_reserves != 0 && d9_reserves != 0 {
                //  let liquidity_check = self.check_new_liquidity(usdt_liquidity, d9_liquidity);
                //  if let Err(e) = liquidity_check {
                //      return Err(e);
                //  }
            }

            let validity_check = self.usdt_validity_check(caller, usdt_liquidity);
            if let Err(e) = validity_check {
                return Err(e);
            }

            // receive usdt from user
            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt_liquidity);
            if receive_usdt_result.is_err() {
                return Err(ContractError::CouldntTransferUSDTFromUser);
            }

            let _ = self.mint_lp_tokens(caller, d9_liquidity, usdt_liquidity)?;

            self.env().emit_event(LiquidityAdded {
                account_id: caller,
                usdt: usdt_liquidity,
                d9: d9_liquidity,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn remove_liquidity(&mut self) -> Result<(), ContractError> {
            let caller = self.env().caller();
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves()?;
            let lp_tokens = {
                let result = self.liquidity_providers.get(&caller);
                match result {
                    None => 0,
                    Some(tokens) => tokens,
                }
            };
            if lp_tokens == 0 {
                return Err(ContractError::LiquidityProviderNotFound);
            }
            // Calculate  contribution
            let liquidity_percent = self.calculate_lp_percent(lp_tokens);
            let d9_liquidity = liquidity_percent.saturating_mul_int(d9_reserves);
            let usdt_liquidity = liquidity_percent.saturating_mul_int(usdt_reserves);
            // get fee portion
            let fee_portion =
                liquidity_percent.saturating_mul(FixedBalance::from_num(self.fee_total));
            self.fee_total = self
                .fee_total
                .saturating_sub(fee_portion.to_num::<Balance>());
            let d9_plus_fee_portion = d9_liquidity.saturating_add(fee_portion);
            // Transfer payouts
            let transfer_result = self
                .env()
                .transfer(caller, d9_plus_fee_portion.to_num::<Balance>());
            if transfer_result.is_err() {
                return Err(ContractError::MarketMakerHasInsufficientFunds(Currency::D9));
            }
            let _ = self.send_usdt_to_user(caller, usdt_liquidity.to_num::<Balance>())?;
            // update liquidity provider
            self.total_lp_tokens = self.total_lp_tokens.saturating_sub(lp_tokens);
            self.liquidity_providers.remove(&caller);

            self.env().emit_event(LiquidityRemoved {
                account_id: caller,
                usdt: usdt_liquidity.to_num::<Balance>(),
                d9: d9_liquidity.to_num::<Balance>(),
            });
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
        fn calculate_lp_percent(&self, lp_tokens: Balance) -> FixedBalance {
            let percent_provided = FixedBalance::from_num(lp_tokens)
                .checked_div(FixedBalance::from_num(self.total_lp_tokens));
            if percent_provided.is_none() {
                return FixedBalance::from_num(0);
            }
            percent_provided.unwrap()
        }

        #[ink(message)]
        pub fn check_new_liquidity(
            &self,
            usdt_liquidity: Balance,
            d9_liquidity: Balance,
        ) -> Result<(), ContractError> {
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves()?;
            let fixed_usdt_reserves = FixedBalance::from_num(usdt_reserves);
            let fixed_d9_reserves = FixedBalance::from_num(d9_reserves);
            let fixed_usdt_liquidity = FixedBalance::from_num(usdt_liquidity);
            let fixed_d9_liquidity = FixedBalance::from_num(d9_liquidity);

            // Use a fixed-point representation for precision, or a library that supports large number arithmetic
            let checked_ratio = fixed_d9_reserves.checked_div(fixed_usdt_reserves);
            let ratio = match checked_ratio {
                Some(r) => r,
                None => {
                    return Err(ContractError::DivisionByZero);
                }
            };

            let checked_threshold_percent =
                FixedBalance::from_num(self.liquidity_tolerance_percent)
                    .checked_div(FixedBalance::from_num(100));
            let threshold_percent = match checked_threshold_percent {
                Some(t) => t,
                None => {
                    return Err(ContractError::DivisionByZero);
                }
            };

            let checked_threshold = threshold_percent.checked_mul(ratio);
            let threshold = match checked_threshold {
                Some(t) => t,
                None => {
                    return Err(ContractError::MultiplicationError);
                }
            };

            let new_ratio = FixedBalance::from_num(
                fixed_d9_reserves
                    .saturating_add(fixed_d9_liquidity)
                    .checked_div(fixed_usdt_reserves.saturating_add(fixed_usdt_liquidity))
                    .unwrap_or(FixedBalance::from_num(0)),
            );

            let price_difference = {
                if new_ratio > ratio {
                    new_ratio.saturating_sub(ratio)
                } else {
                    ratio.saturating_sub(new_ratio)
                }
            };

            if threshold < price_difference {
                return Err(ContractError::LiquidityAddedBeyondTolerance(
                    threshold.to_num::<Balance>(),
                    price_difference.to_num::<Balance>(),
                ));
            }
            Ok(())
        }
        //   fn calculate_price(&self, amount: Balance) -> Balance {}
        /// sell usdt
        #[ink(message)]
        pub fn get_d9(&mut self, usdt: Balance) -> Result<Balance, ContractError> {
            let caller: AccountId = self.env().caller();

            // receive sent usdt from caller
            let check_user_result = self.check_usdt_allowance(caller, usdt.clone());
            if check_user_result.is_err() {
                return Err(check_user_result.unwrap_err());
            }

            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt.clone());
            if receive_usdt_result.is_err() {
                return Err(ContractError::CouldntTransferUSDTFromUser);
            }

            //prepare d9 to send
            let d9_calc_result =
                self.calculate_exchange(Direction(Currency::USDT, Currency::D9), usdt);
            if let Err(e) = d9_calc_result {
                return Err(e);
            }
            let d9 = d9_calc_result.unwrap();
            let transaction_fee = self.calc_fee(d9);
            let d9_minus_fee = d9.saturating_sub(transaction_fee);

            // send d9
            // let fee: Balance = self.calculate_fee(&d9)?;
            // let d9_minus_fee = d9.saturating_sub(fee);
            let transfer_result = self.env().transfer(caller, d9_minus_fee);
            if transfer_result.is_err() {
                return Err(ContractError::MarketMakerHasInsufficientFunds(Currency::D9));
            }

            self.env().emit_event(USDTToD9Conversion {
                account_id: caller,
                usdt,
                d9: d9_minus_fee,
            });

            Ok(d9)
        }

        /// sell d9
        #[ink(message, payable)]
        pub fn get_usdt(&mut self) -> Result<Balance, ContractError> {
            let direction = Direction(Currency::D9, Currency::USDT);
            // calculate amount
            let d9: Balance = self.env().transferred_value();
            // let fee: Balance = self.calculate_fee(d9)?;
            // let amount_minus_fee = d9.saturating_sub(fee);
            let usdt_calc_result = self.calculate_exchange(direction, d9);
            if usdt_calc_result.is_err() {
                return Err(usdt_calc_result.unwrap_err());
            }
            let usdt = usdt_calc_result.unwrap();
            //prepare to send
            let is_balance_sufficient = self.check_usdt_balance(self.env().account_id(), usdt);
            if is_balance_sufficient.is_err() {
                return Err(ContractError::InsufficientLiquidity(Currency::USDT));
            }

            // send usdt
            let caller = self.env().caller();
            let _ = self.send_usdt_to_user(caller, usdt.clone())?;

            self.env().emit_event(D9ToUSDTConversion {
                account_id: caller,
                usdt,
                d9,
            });

            Ok(usdt)
        }

        /// mint lp tokens, credit provider account
        fn mint_lp_tokens(
            &mut self,
            provider_id: AccountId,
            new_d9_liquidity: Balance,
            new_usdt_liquidity: Balance,
        ) -> Result<(), ContractError> {
            let provider_current_lp = self
                .liquidity_providers
                .get(&provider_id)
                .unwrap_or_default();

            let new_lp_tokens = self.calc_new_lp_tokens(new_d9_liquidity, new_usdt_liquidity)?;

            if new_lp_tokens == 0 {
                return Err(ContractError::LiquidityTooLow);
            }
            //add tokens to lp provider and contract total
            self.total_lp_tokens = self.total_lp_tokens.saturating_add(new_lp_tokens);

            let updated_provider_lp = provider_current_lp.saturating_add(new_lp_tokens);

            self.liquidity_providers
                .insert(provider_id, &updated_provider_lp);

            Ok(())
        }

        /// calculate lp tokens based on usdt liquidity
        #[ink(message)]
        pub fn calc_new_lp_tokens(
            &mut self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance,
        ) -> Result<Balance, ContractError> {
            // Initialize LP tokens if the pool is empty
            if self.total_lp_tokens == 0 {
                return Ok(1_000_000);
            }
            // Get current reserves
            let (d9_reserve, usdt_reserve) = self.get_currency_reserves()?;
            let current_reserve_total = d9_reserve.saturating_add(usdt_reserve);
            let new_liquidity_total = d9_liquidity.saturating_add(usdt_liquidity);
            let new_liquidity_ratio = FixedBalance::from_num(new_liquidity_total)
                .checked_div(FixedBalance::from_num(current_reserve_total))
                .unwrap_or(FixedBalance::from_num(0));
            let new_lp_tokens = new_liquidity_ratio.saturating_mul_int(self.total_lp_tokens);
            Ok(new_lp_tokens.to_num::<Balance>())
        }

        fn usdt_validity_check(
            &self,
            caller: AccountId,
            amount: Balance,
        ) -> Result<(), ContractError> {
            // does sender have sufficient usdt
            let _ = self.check_usdt_balance(caller, amount)?;
            // did sender provider sufficient allowance permission
            let _ = self.check_usdt_allowance(caller, amount)?;
            Ok(())
        }

        /// amount of currency B from A, if A => B
        #[ink(message)]
        pub fn calculate_exchange(
            &self,
            direction: Direction,
            amount_0: Balance,
        ) -> Result<Balance, ContractError> {
            //naming comes from Direction. e.g. direction.0 is the first currency in the pair
            // get currency balances
            let balance_0: Balance = self.get_currency_balance(direction.0)?;
            let balance_1: Balance = self.get_currency_balance(direction.1)?;

            // liquidity checks
            if balance_1 == 0 {
                return Err(ContractError::InsufficientLiquidity(direction.1));
            }
            self.calc_opposite_currency_amount(balance_0, balance_1, amount_0)
        }

        #[ink(message)]
        pub fn estimate_exchange(
            &self,
            direction: Direction,
            amount_0: Balance,
        ) -> Result<(Balance, Balance), ContractError> {
            let balance_0: Balance = self.get_currency_balance(direction.0)?;
            let balance_1: Balance = self.get_currency_balance(direction.1)?;

            // liquidity checks
            if balance_1 == 0 {
                return Err(ContractError::InsufficientLiquidity(direction.1));
            }
            let amount_1 = self.calc_opposite_currency_amount(
                balance_0.saturating_add(amount_0),
                balance_1,
                amount_0,
            )?;
            Ok((amount_0, amount_1))
        }

        pub fn calc_opposite_currency_amount(
            &self,
            balance_0: Balance,
            balance_1: Balance,
            amount_0: Balance,
        ) -> Result<Balance, ContractError> {
            let fixed_balance_0 = FixedBalance::from_num(balance_0);
            let fixed_balance_1 = FixedBalance::from_num(balance_1);
            let fixed_amount_0 = FixedBalance::from_num(amount_0);
            let fixed_curve_k = fixed_balance_0.saturating_mul(fixed_balance_1);
            let new_balance_0 = fixed_balance_0.saturating_add(fixed_amount_0);
            let new_balance_1_opt = fixed_curve_k.checked_div(new_balance_0);
            if new_balance_1_opt.is_none() {
                return Err(ContractError::DivisionByZero);
            }
            let new_balance_1 = new_balance_1_opt.unwrap();
            let amount_1 = fixed_balance_1.saturating_sub(new_balance_1);
            Ok(amount_1.to_num::<Balance>())
        }

        fn calc_fee(&self, amount: Balance) -> Balance {
            let fee_percent = Perquintill::from_parts(self.fee_percent as u64);
            fee_percent.mul_floor(amount)
        }

        fn get_currency_balance(&self, currency: Currency) -> Result<Balance, ContractError> {
            match currency {
                Currency::D9 => Ok(self.env().balance()),
                Currency::USDT => self.get_usdt_balance(self.env().account_id()),
            }
        }

        /// check if usdt balance is sufficient for swap
        #[ink(message)]
        pub fn check_usdt_balance(
            &self,
            account_id: AccountId,
            amount: Balance,
        ) -> Result<(), ContractError> {
            let usdt_balance = self.get_usdt_balance(account_id)?;
            if usdt_balance < amount {
                return Err(ContractError::USDTBalanceInsufficient);
            }
            Ok(())
        }

        pub fn get_usdt_balance(&self, account_id: AccountId) -> Result<Balance, ContractError> {
            let cross_call_attempt = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::balance_of")))
                        .push_arg(account_id),
                )
                .returns::<Balance>()
                .try_invoke();
            if cross_call_attempt.is_err() {
                return Err(cross_call_attempt.err().unwrap().into());
            }
            let result = cross_call_attempt.unwrap();
            match result {
                Ok(balance) => Ok(balance),
                Err(e) => Err(ContractError::LangError(e)),
            }
        }

        pub fn check_usdt_allowance(
            &self,
            owner: AccountId,
            amount: Balance,
        ) -> Result<(), ContractError> {
            let cross_call_attempt = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::allowance")))
                        .push_arg(owner)
                        .push_arg(self.env().account_id()),
                )
                .returns::<Balance>()
                .try_invoke();
            if cross_call_attempt.is_err() {
                return Err(cross_call_attempt.err().unwrap().into());
            }
            let result = cross_call_attempt.unwrap();
            match result {
                Ok(allowance) => {
                    if allowance < amount {
                        return Err(ContractError::USDTAllowanceInsufficient);
                    } else {
                        Ok(())
                    }
                }
                Err(e) => return Err(ContractError::LangError(e)),
            }
        }

        pub fn send_usdt_to_user(
            &self,
            recipient: AccountId,
            amount: Balance,
        ) -> Result<(), ContractError> {
            let cross_call_result = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer")))
                        .push_arg(recipient)
                        .push_arg(amount)
                        .push_arg([0u8]),
                )
                .returns::<Result<(), ContractError>>()
                .try_invoke();
            if cross_call_result.is_err() {
                return Err(cross_call_result.err().unwrap().into());
            }
            let contract_result = cross_call_result.unwrap();
            match contract_result {
                Ok(_) => Ok(()),
                Err(e) => Err(ContractError::LangError(e)),
            }
        }

        pub fn receive_usdt_from_user(
            &self,
            sender: AccountId,
            amount: Balance,
        ) -> Result<(), ContractError> {
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
                .returns::<Result<(), ContractError>>()
                .invoke()
        }

        fn only_admin(&self) -> () {
            assert!(
                self.env().caller() == self.admin,
                "Only admin can change admin."
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::test::default_accounts;
        use substrate_fixed::{types::extra::U6, FixedU128};
        type FixedBalance = FixedU128<U6>;
        use sp_arithmetic::Perquintill;
        //   #[ink::test]
        //   fn can_build() {
        //       let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
        //       let usdt_contract = default_accounts().alice;
        //       let mut market_maker = MarketMaker::new(usdt_contract, 4, 100, 8);
        //       assert!(market_maker.usdt_contract == usdt_contract);
        //   }

        //   fn default_contract() -> MarketMaker {
        //       let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
        //       let usdt_contract = default_accounts().alice;
        //       let mut market_maker = MarketMaker::new(usdt_contract, 4, 100, 8);
        //       market_maker.total_lp_tokens = 1_000_000;
        //       market_maker
        //   }
        #[ink::test]
        fn check_new_liquidity() {
            let d9_liquidity: Balance = 10000_000_000_000_000;
            let usdt_liquidity: Balance = 8500_00;
            let (d9_reserves, usdt_reserves): (Balance, Balance) = (100_000_000_000_000, 100_00);

            let ratio = d9_reserves.saturating_div(usdt_reserves);
            let threshold_percent = Perquintill::from_percent(10);

            let threshold = threshold_percent.mul_floor(ratio);
            println!("threshold: {}", threshold);
            let new_ratio = d9_reserves
                .saturating_add(d9_liquidity)
                .saturating_div(usdt_reserves.saturating_add(usdt_liquidity));
            println!("new ratio: {}", new_ratio);
            let price_difference = {
                if ratio > new_ratio {
                    ratio.saturating_sub(new_ratio)
                } else {
                    new_ratio.saturating_sub(ratio)
                }
            };
            println!("price difference: {}", price_difference);

            assert!(price_difference < threshold)
        }
        //   #[ink::test]
        //   fn new_liquidity_is_within_threshold_range() {
        //       //setup contract
        //       let market_maker = default_contract();

        //       // new liquidity
        //       let usdt_liquidity = 1_000_000;
        //       let d9_liquidity = 1_000_000;

        //       let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
        //       assert!(result.is_ok());
        //   }

        //   #[ink::test]
        //   fn new_liquidity_is_below_threshold_range() {
        //       //setup contract
        //       let market_maker = default_contract();

        //       // new liquidity
        //       let usdt_liquidity = 9_000_000;
        //       let d9_liquidity = 1_000_000;

        //       let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
        //       assert!(result.is_err());
        //   }

        //   #[ink::test]
        //   fn new_liquidity_is_above_threshold_range() {
        //       //setup contract
        //       let market_maker = default_contract();

        //       // new liquidity
        //       let usdt_liquidity = 3_000_000;
        //       let d9_liquidity = 13_000_000;

        //       let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
        //       assert!(result.is_err());
        //   }

        //   #[ink::test]
        //   fn calc_new_lp_tokens_initial_value() {
        //       let mut market_maker = default_contract();
        //       market_maker.total_lp_tokens = 0; //default is 1_000_000
        //       let new_usdt_liquidity = 10_000_000;
        //       let new_d9_tokens = 1_000_000;
        //       let new_lp_tokens = market_maker.calc_new_lp_tokens(new_usdt_liquidity, new_d9_tokens);
        //       assert!(new_lp_tokens == 1_000_000);
        //   }

        //   #[ink::test]
        //   fn calc_new_lp_tokens_value() {
        //       let mut market_maker = default_contract();
        //       let new_usdt_liquidity = 1_000_000_000;
        //       let new_d9_tokens = 1_000_000_000;
        //       let new_lp_tokens = market_maker.calc_new_lp_tokens(new_d9_tokens, new_usdt_liquidity);
        //       assert_eq!(new_lp_tokens, 1_000_000);
        //   }
        //   #[ink::test]
        //   fn calc_new_lp_tokens_value_alt() {
        //       let mut market_maker = default_contract();
        //       let new_usdt_liquidity = 780_000;
        //       let new_d9_tokens = 1_000_000;
        //       let new_lp_tokens = market_maker.calc_new_lp_tokens(new_usdt_liquidity, new_d9_tokens);
        //       assert_eq!(new_lp_tokens, 890);
        //   }

        //   #[ink::test]
        //   fn mint_lp_tokens() {
        //       let mut market_maker = default_contract();
        //       let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
        //       let test_account = default_accounts().alice;
        //       let liquidity_provider = LiquidityProvider {
        //           account_id: test_account,
        //           usdt: 0,
        //           d9: 0,
        //           lp_tokens: 0,
        //       };
        //       market_maker.liquidity_providers.insert(test_account, &liquidity_provider);
        //       let previous_maker_lp_tokens = market_maker.total_lp_tokens;
        //       let usdt_liquidity = 10_000_000_000;
        //       let d9_liquidity = 10_000_000_000;
        //       let calculated_lp_tokens = market_maker.calc_new_lp_tokens(
        //           d9_liquidity,
        //           usdt_liquidity
        //       );
        //       println!("calculated lp tokens: {}", calculated_lp_tokens);
        //       market_maker.mint_lp_tokens(d9_liquidity, usdt_liquidity, liquidity_provider);

        //       let retrieved_provider = market_maker.get_liquidity_provider(test_account).unwrap();
        //       assert_eq!(calculated_lp_tokens, retrieved_provider.lp_tokens, "incorrect lp tokens");
        //       assert_eq!(
        //           market_maker.total_lp_tokens,
        //           previous_maker_lp_tokens.saturating_add(calculated_lp_tokens),
        //           "tokens not saved"
        //       );
        //   }
    }

    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        use super::*;
        use d9_usdt::d9_usdt::D9USDTRef;
        use d9_usdt::d9_usdt::D9USDT;
        use ink_e2e::{account_id, build_message, AccountKeyring};
        //   use openbrush::contracts::psp22::psp22_external::PSP22;
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        #[ink_e2e::test]
        async fn check_liquidity(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let d9_liquidity: Balance = 10_000_000000000000;
            let usdt_liquidity: Balance = 10_000_00;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 10);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let caller = account_id(AccountKeyring::Alice);
            let check_liquidity_message = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|market_maker| {
                    market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity)
                });

            let response = client
                .call(&ink_e2e::alice(), check_liquidity_message, 0, None)
                .await;
            // execute approval call
            assert!(response.is_ok());
            Ok(())
        }
        #[ink_e2e::test]
        async fn check_usdt_balance(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let caller = account_id(AccountKeyring::Alice);
            let check_usdt_balance_message = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|market_maker| {
                    market_maker.check_usdt_balance(caller, initial_supply.saturating_div(2000))
                });

            let response = client
                .call(&ink_e2e::alice(), check_usdt_balance_message, 0, None)
                .await;
            // execute approval call
            assert!(response.is_ok());
            Ok(())
        }

        #[ink_e2e::test]
        async fn check_usdt_allowance(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            //init usdt contract
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let usdt_approved_amount = initial_supply.saturating_div(2000);
            let approval_message =
                build_message::<D9USDTRef>(usdt_address.clone()).call(|d9_usdt| {
                    d9_usdt.approve(
                        ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                        amm_address.clone(),
                        usdt_approved_amount,
                    )
                });
            // execute approval call
            let approval_response = client
                .call(&ink_e2e::alice(), approval_message, 0, None)
                .await;
            assert!(approval_response.is_ok());

            //check allowance
            let caller = account_id(AccountKeyring::Alice);
            let check_usdt_allowance_message = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|market_maker| {
                    market_maker
                        .check_usdt_allowance(caller, usdt_approved_amount.saturating_div(10))
                });

            let response = client
                .call(&ink_e2e::alice(), check_usdt_allowance_message, 0, None)
                .await;
            // execute approval call
            assert!(response.is_ok());
            Ok(())
        }
        //   #[ink_e2e::test]
        //   async fn receive_usdt_from_user(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
        //       //init usdt contract
        //       let initial_supply: Balance = 100_000_000_000_000;
        //       let usdt_constructor = D9USDTRef::new(initial_supply);
        //       let usdt_address = client
        //           .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
        //           .expect("failed to instantiate usdt").account_id;
        //       // init market maker
        //       let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
        //       let amm_address = client
        //           .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None).await
        //           .expect("failed to instantiate market maker").account_id;

        //       //build approval message
        //       let usdt_approved_amount = initial_supply.saturating_div(2000);
        //       let approval_message = build_message::<D9USDTRef>(usdt_address.clone()).call(|d9_usdt|
        //           d9_usdt.approve(
        //               ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
        //               amm_address.clone(),
        //               usdt_approved_amount
        //           )
        //       );
        //       // execute approval call
        //       let approval_response = client.call(&ink_e2e::alice(), approval_message, 0, None).await;
        //       assert!(approval_response.is_ok());

        //       let caller = account_id(AccountKeyring::Alice);
        //       let receive_from_user_message = build_message::<MarketMakerRef>(
        //           amm_address.clone()
        //       ).call(|market_maker|
        //           market_maker.receive_usdt_from_user(caller, usdt_approved_amount.saturating_div(10))
        //       );

        //       let response = client.call(&ink_e2e::alice(), receive_from_user_message, 0, None).await;
        //       // execute approval call
        //       assert!(response.is_ok());
        //       Ok(())
        //   }
        #[ink_e2e::test]
        async fn add_liquidity(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            //init usdt contract
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let usdt_approval_amount = 100_000_000_000_000;
            let approval_message =
                build_message::<D9USDTRef>(usdt_address.clone()).call(|d9_usdt| {
                    d9_usdt.approve(
                        ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                        amm_address.clone(),
                        usdt_approval_amount,
                    )
                });
            // execute approval call
            let approval_response = client
                .call(&ink_e2e::alice(), approval_message, 0, None)
                .await;
            assert!(approval_response.is_ok());

            // add liquidity
            let usdt_liquidity_amount = usdt_approval_amount.saturating_div(20);
            let d9_liquidity_amount = usdt_liquidity_amount.saturating_div(10);
            let add_liquidity_message = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|market_maker| market_maker.add_liquidity(usdt_liquidity_amount));
            let add_liquidity_response = client
                .call(
                    &ink_e2e::alice(),
                    add_liquidity_message,
                    d9_liquidity_amount,
                    None,
                )
                .await;

            assert!(add_liquidity_response.is_ok());
            Ok(())
        }
        // setup default contracts
    }
} //---LAST LINE OF IMPLEMENTATION OF THE INK! SMART CONTRACT---//
