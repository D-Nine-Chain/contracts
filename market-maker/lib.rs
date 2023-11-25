#![cfg_attr(not(feature = "std"), no_std, no_main)]
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod market_maker {
    use super::*;
    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use ink::selector_bytes;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use substrate_fixed::{ FixedU128, types::extra::U6 };
    type FixedBalance = FixedU128<U6>;
    #[ink(storage)]
    pub struct MarketMaker {
        /// contract for usdt coin
        usdt_contract: AccountId,
        /// Perbill::from_rational(fee_numerator, fee_denominator)
        fee_numerator: u32,
        /// Perbill::from_rational(fee_numerator, fee_denominator)
        fee_denominator: u32,
        /// total fees collected
        fee_total: Balance,
        ///represents numerator of a percent
        liquidity_tolerance_percent: u32,
        /// providers of contract liquidity
        liquidity_providers: Mapping<AccountId, LiquidityProvider>,
        /// total number of liquidity pool tokens
        lp_tokens: Balance,
    }

    #[derive(scale::Decode, scale::Encode, Clone)]
    #[cfg_attr(
        feature = "std",
        derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
    )]
    pub struct LiquidityProvider {
        account_id: AccountId,
        usdt: Balance,
        d9: Balance,
        lp_tokens: Balance,
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
    pub struct CurrencySwap {
        // initiator of swap
        #[ink(topic)]
        account_id: AccountId,
        // from => to
        #[ink(topic)]
        direction: (Direction, Direction),
        // time of execution
        #[ink(topic)]
        time: Timestamp,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        ConversionAmountTooLow,
        InsufficientLiquidity(Currency),
        InsufficientAllowance,
        MarketMakerHasInsufficientFunds(Currency),
        InsufficientLiquidityProvided,
        USDTBalanceInsufficient,
        LiquidityProviderNotFound,
        InsufficientLPTokens,
        InsufficientContractLPTokens,
        DivisionByZero,
        USDTTooSmall,
        USDTTooMuch,
    }

    impl MarketMaker {
        #[ink(constructor)]
        pub fn new(
            usdt_contract: AccountId,
            fee_numerator: u32,
            fee_denominator: u32,
            liquidity_tolerance_percent: u32
        ) -> Self {
            assert!(
                0 < liquidity_tolerance_percent && liquidity_tolerance_percent <= 100,
                "tolerance must be 0 < x <= 100"
            );
            Self {
                usdt_contract,
                fee_numerator,
                fee_denominator,
                fee_total: Default::default(),
                liquidity_tolerance_percent,
                liquidity_providers: Default::default(),
                lp_tokens: Default::default(),
            }
        }

        /// get pool balances (d9, usdt)
        #[ink(message)]
        pub fn get_currency_reserves(&self) -> (Balance, Balance) {
            let d9_balance: Balance = self.env().balance();
            let usdt_balance: Balance = self.get_usdt_balance(self.env().account_id());
            (d9_balance, usdt_balance)
        }

        #[ink(message)]
        pub fn get_liquidity_provider(&self, account_id: AccountId) -> Option<LiquidityProvider> {
            self.liquidity_providers.get(&account_id)
        }
        /// add liquidity by adding tokens to the reserves
        #[ink(message, payable)]
        pub fn add_liquidity(&mut self, usdt_liquidity: Balance) -> Result<(), Error> {
            //get liquidity provider info
            let caller = self.env().caller();
            let liquidity_provider_result = self.liquidity_providers.get(&caller);
            let liquidity_provider: LiquidityProvider = match liquidity_provider_result {
                Some(liquidity_provider) => liquidity_provider,
                None => {
                    let liquidity_provider = LiquidityProvider {
                        account_id: caller,
                        usdt: 0,
                        d9: 0,
                        lp_tokens: 0,
                    };
                    liquidity_provider
                }
            };

            let d9_liquidity = self.env().transferred_value();

            // greeater than zero checks
            if usdt_liquidity.clone() == 0 || d9_liquidity.clone() == 0 {
                return Err(Error::InsufficientLiquidityProvided);
            }

            let (mut d9_reserves, mut usdt_reserves) = self.get_currency_reserves();
            // if usdt_reserves != 0 && d9_reserves != 0 {
            //     let liquidity_check = self.check_new_liquidity(d9_liquidity, usdt_liquidity);
            //     if liquidity_check.is_err() {
            //         return Err(liquidity_check.unwrap_err());
            //     }
            // }
            // make sure new liquidity doesn't deviate price more than tolerance

            // does sender have sufficient usdt
            let usdt_balance_check_result = self.check_usdt_balance(caller, usdt_liquidity.clone());
            if usdt_balance_check_result.is_err() {
                return Err(usdt_balance_check_result.unwrap_err());
            }

            // did sender provider sufficient allowance permission
            let usdt_allowance_check = self.check_usdt_allowance(caller, usdt_liquidity);
            if usdt_allowance_check.is_err() {
                return Err(usdt_allowance_check.unwrap_err());
            }

            // receive usdt from user
            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt_liquidity);
            if receive_usdt_result.is_err() {
                return Err(receive_usdt_result.unwrap_err());
            }

            // self.mint_lp_tokens(d9_liquidity, usdt_liquidity.clone(), liquidity_provider.clone());

            Ok(())
        }

        #[ink(message)]
        pub fn remove_liquidity(&mut self) -> Result<(), Error> {
            let caller = self.env().caller();
            let (mut d9_reserves, mut usdt_reserves) = self.get_reserves();

            let liquidity_provider = self.liquidity_providers.get(&caller);
            if liquidity_provider.is_none() {
                return Err(Error::LiquidityProviderNotFound);
            }
            // Calculate percentage contributed
            let d9_percent = liquidity_provider.d9 / d9_reserves;
            let usdt_percent = liquidity_provider.usdt / usdt_reserves;

            // Pay out respective percentages of current reserves
            let d9_payout = d9_reserves * d9_percent;
            let usdt_payout = usdt_reserves * usdt_percent;

            d9_reserves -= d9_payout;
            usdt_reserves -= usdt_payout;

            // Transfer payouts
            self.env().transfer(caller, d9_payout)?;
            self.send_usdt(caller, usdt_payout)?;

            // Remove liquidity
            self.lp_tokens -= liquidity_provider.lp_tokens;
            self.liquidity_providers.remove(&caller);

            Ok(())
        }
        //   #[ink(message)]
        //   pub fn remove_liquidity(&mut self, usdt: Balance) -> Result<(), Error> {}

        fn conform_d9_to_u32(d9: Balance) -> u32 {
            d9.saturating_div(1_000_000_000_000) as u32
        }
        /// ensure added liquidity will not deviate price more than tolerance
        #[ink(message)]
        pub fn check_new_liquidity(
            &self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance
        ) -> Result<(), Error> {
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();
            let usdt_per_d9_calc: Option<FixedBalance> = FixedBalance::from_num(
                usdt_reserves
            ).checked_div(FixedBalance::from_num(d9_reserves));
            if usdt_per_d9_calc.is_none() {
                return Err(Error::DivisionByZero);
            }
            let usdt_per_d9 = usdt_per_d9_calc.unwrap();
            let ideal_usdt_liquidity = usdt_per_d9.saturating_mul_int(d9_liquidity);
            // let allowed_deviation_fraction = Perbill::from_percent(
            //     self.liquidity_tolerance_percent
            // );
            let allowed_deviation_fraction = FixedBalance::from_num(
                self.liquidity_tolerance_percent
            )
                .checked_div(FixedBalance::from_num(100))
                .unwrap();
            let allowed_deviation_amount =
                allowed_deviation_fraction.saturating_mul(ideal_usdt_liquidity);
            // // Calculate bounds for d9_liquidity using saturating arithmetic
            let min_usdt = ideal_usdt_liquidity.saturating_sub(allowed_deviation_amount);
            let max_usdt = ideal_usdt_liquidity.saturating_add(allowed_deviation_amount);

            // // Check if the provided d9_liquidity is within bounds
            if usdt_liquidity < min_usdt {
                return Err(Error::USDTTooSmall);
            }
            if max_usdt < usdt_liquidity {
                return Err(Error::USDTTooMuch);
            }
            Ok(())
        }
        //   fn calculate_price(&self, amount: Balance) -> Balance {}
        /// sell usdt
        #[ink(message)]
        pub fn get_d9(&mut self, usdt: Balance) -> Result<(Currency, Balance), Error> {
            let caller: AccountId = self.env().caller();

            // receive sent usdt from caller
            let check_user_result = self.check_usdt_allowance(caller, usdt.clone());
            if check_user_result.is_err() {
                return Err(check_user_result.unwrap_err());
            }

            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt.clone());
            if receive_usdt_result.is_err() {
                return Err(receive_usdt_result.unwrap_err());
            }

            //prepare d9 to send
            let d9_calc_result = self.calculate_exchange(
                &Direction(Currency::USDT, Currency::D9),
                usdt
            );
            if d9_calc_result.is_err() {
                return Err(d9_calc_result.unwrap_err());
            }
            let d9 = d9_calc_result.unwrap();
            // send d9
            // let fee: Balance = self.calculate_fee(&d9)?;
            // let d9_minus_fee = d9.saturating_sub(fee);
            let transfer_result = self.env().transfer(caller, d9);
            if transfer_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::D9));
            }

            Ok((Currency::D9, d9))
        }

        /// sell d9
        #[ink(message, payable)]
        pub fn get_usdt(&mut self) -> Result<(Currency, Balance), Error> {
            let caller: AccountId = self.env().caller();

            let direction = Direction(Currency::D9, Currency::USDT);
            // calculate amount
            let d9: Balance = self.env().transferred_value();
            // let fee: Balance = self.calculate_fee(d9)?;
            // let amount_minus_fee = d9.saturating_sub(fee);
            let usdt_calc_result = self.calculate_exchange(&direction, d9);
            if usdt_calc_result.is_err() {
                return Err(usdt_calc_result.unwrap_err());
            }
            let usdt = usdt_calc_result.unwrap();
            //prepare to send
            let is_balance_sufficient = self.check_usdt_balance(self.env().account_id(), usdt);
            if is_balance_sufficient.is_err() {
                return Err(Error::InsufficientLiquidity(Currency::USDT));
            }

            // send usdt
            self.send_usdt_to_user(caller, usdt.clone())?;

            Ok((Currency::USDT, usdt))
        }

        /// mint lp tokens, credit provider account
        fn mint_lp_tokens(
            &mut self,
            new_d9_liquidity: Balance,
            new_usdt_liquidity: Balance,
            mut liquidity_provider: LiquidityProvider
        ) {
            let new_lp_tokens = self.calc_new_lp_tokens(new_d9_liquidity, new_usdt_liquidity);
            //add tokens to lp provider and contract total
            self.lp_tokens = self.lp_tokens.saturating_add(new_lp_tokens);
            liquidity_provider.lp_tokens =
                liquidity_provider.lp_tokens.saturating_add(new_lp_tokens);

            liquidity_provider.d9 = liquidity_provider.d9.saturating_add(new_d9_liquidity);
            liquidity_provider.usdt = liquidity_provider.usdt.saturating_add(new_usdt_liquidity);

            self.liquidity_providers.insert(liquidity_provider.account_id, &liquidity_provider);
        }

        /// calculate lp tokens based on usdt liquidity
        #[ink(message)]
        pub fn calc_new_lp_tokens(
            &mut self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance
        ) -> Balance {
            if self.lp_tokens == 0 {
                return 1_000_000;
            }
            let (d9_reserve, usdt_reserve) = self.get_currency_reserves();
            let current_reserve_total = d9_reserve.saturating_add(usdt_reserve);
            if current_reserve_total == 0 {
                return 1_000_000;
            }
            let new_liquidity_total = d9_liquidity.saturating_add(usdt_liquidity);
            let percent_of_pool_calc: Option<FixedBalance> = FixedBalance::from_num(
                new_liquidity_total
            ).checked_div(FixedBalance::from_num(current_reserve_total + new_liquidity_total));
            if percent_of_pool_calc.is_none() {
                return 1_000_000;
            }
            let percent_of_pool = percent_of_pool_calc.unwrap();
            let new_lp_tokens = percent_of_pool.saturating_mul_int(self.lp_tokens);
            new_lp_tokens.to_num::<Balance>()
        }

        /// amount of currency B from A, if A => B
        fn calculate_exchange(
            &self,
            direction: &Direction,
            amount_0: Balance
        ) -> Result<Balance, Error> {
            //naming comes from Direction. e.g. direction.0 is the first currency in the pair
            // get currency balances
            let balance_0: Balance = self.get_currency_balance(direction.0);
            let balance_1: Balance = self.get_currency_balance(direction.1);
            let curve_k = balance_0.saturating_mul(balance_1);

            // liquidity checks
            if balance_1 == 0 {
                return Err(Error::InsufficientLiquidity(direction.1));
            }

            let new_balance_0: Balance = balance_0.saturating_add(amount_0);

            let new_balance_1: Balance = curve_k.saturating_div(new_balance_0);

            let amount_1: Balance = balance_1.saturating_sub(new_balance_1);

            Ok(amount_1)
        }

        fn get_currency_balance(&self, currency: Currency) -> Balance {
            match currency {
                Currency::D9 => self.env().balance(),
                Currency::USDT => self.get_usdt_balance(self.env().account_id()),
            }
        }
        /// get exchange fee
        //   fn calculate_fee(&self, amount: Balance) -> Result<Balance, Error> {
        //       let fee: Balance = percent.mul_ceil(amount);
        //       if fee == 0 {
        //           return Err(Error::ConversionAmountTooLow);
        //       }
        //       Ok(fee)
        //   }

        /// check if usdt balance is sufficient for swap
        #[ink(message)]
        pub fn check_usdt_balance(
            &self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let usdt_balance = self.get_usdt_balance(account_id);

            if usdt_balance < amount {
                return Err(Error::USDTBalanceInsufficient);
            }
            Ok(())
        }
        #[ink(message)]
        pub fn get_usdt_balance(&self, account_id: AccountId) -> Balance {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("PSP22::balance_of"))
                    ).push_arg(account_id)
                )
                .returns::<Balance>()
                .invoke()
        }
        #[ink(message)]
        pub fn check_usdt_allowance(&self, owner: AccountId, amount: Balance) -> Result<(), Error> {
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
        #[ink(message)]
        pub fn send_usdt_to_user(
            &self,
            recipient: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
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
            let _ = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer_from")))
                        .push_arg(sender)
                        .push_arg(self.env().account_id())
                        .push_arg(amount)
                        .push_arg([0u8])
                )
                .returns::<()>()
                .invoke();
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::test::default_accounts;

        #[ink::test]
        fn can_build() {
            let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
            let usdt_contract = default_accounts().alice;
            let mut market_maker = MarketMaker::new(usdt_contract, 4, 100, 8);
            assert!(market_maker.usdt_contract == usdt_contract);
        }

        fn default_contract() -> MarketMaker {
            let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
            let usdt_contract = default_accounts().alice;
            let mut market_maker = MarketMaker::new(usdt_contract, 4, 100, 8);
            market_maker.lp_tokens = 1_000_000;
            market_maker
        }

        #[ink::test]
        fn new_liquidity_is_within_threshold_range() {
            //setup contract
            let market_maker = default_contract();

            // new liquidity
            let usdt_liquidity = 1_000_000;
            let d9_liquidity = 1_000_000;

            let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
            assert!(result.is_ok());
        }

        #[ink::test]
        fn new_liquidity_is_below_threshold_range() {
            //setup contract
            let market_maker = default_contract();

            // new liquidity
            let usdt_liquidity = 9_000_000;
            let d9_liquidity = 1_000_000;

            let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
            assert!(result.is_err());
        }

        #[ink::test]
        fn new_liquidity_is_above_threshold_range() {
            //setup contract
            let market_maker = default_contract();

            // new liquidity
            let usdt_liquidity = 3_000_000;
            let d9_liquidity = 13_000_000;

            let result = market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity);
            assert!(result.is_err());
        }

        #[ink::test]
        fn calc_new_lp_tokens_initial_value() {
            let mut market_maker = default_contract();
            market_maker.lp_tokens = 0; //default is 1_000_000
            let new_usdt_liquidity = 10_000_000;
            let new_d9_tokens = 1_000_000;
            let new_lp_tokens = market_maker.calc_new_lp_tokens(new_usdt_liquidity, new_d9_tokens);
            assert!(new_lp_tokens == 1_000_000);
        }

        #[ink::test]
        fn calc_new_lp_tokens_value() {
            let mut market_maker = default_contract();
            let new_usdt_liquidity = 1_000_000_000;
            let new_d9_tokens = 1_000_000_000;
            let new_lp_tokens = market_maker.calc_new_lp_tokens(new_d9_tokens, new_usdt_liquidity);
            assert_eq!(new_lp_tokens, 1_000_000);
        }
        #[ink::test]
        fn calc_new_lp_tokens_value_alt() {
            let mut market_maker = default_contract();
            let new_usdt_liquidity = 780_000;
            let new_d9_tokens = 1_000_000;
            let new_lp_tokens = market_maker.calc_new_lp_tokens(new_usdt_liquidity, new_d9_tokens);
            assert_eq!(new_lp_tokens, 890);
        }

        #[ink::test]
        fn mint_lp_tokens() {
            let mut market_maker = default_contract();
            let default_accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>;
            let test_account = default_accounts().alice;
            let liquidity_provider = LiquidityProvider {
                account_id: test_account,
                usdt: 0,
                d9: 0,
                lp_tokens: 0,
            };
            market_maker.liquidity_providers.insert(test_account, &liquidity_provider);
            let previous_maker_lp_tokens = market_maker.lp_tokens;
            let usdt_liquidity = 10_000_000_000;
            let d9_liquidity = 10_000_000_000;
            let calculated_lp_tokens = market_maker.calc_new_lp_tokens(
                d9_liquidity,
                usdt_liquidity
            );
            println!("calculated lp tokens: {}", calculated_lp_tokens);
            market_maker.mint_lp_tokens(d9_liquidity, usdt_liquidity, liquidity_provider);

            let retrieved_provider = market_maker.get_liquidity_provider(test_account).unwrap();
            assert_eq!(calculated_lp_tokens, retrieved_provider.lp_tokens, "incorrect lp tokens");
            assert_eq!(
                market_maker.lp_tokens,
                previous_maker_lp_tokens.saturating_add(calculated_lp_tokens),
                "tokens not saved"
            );
        }
    }

    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        use super::*;
        use ink_e2e::{ build_message, account_id, AccountKeyring };
        use d9_usdt::d9_usdt::D9USDT;
        use d9_usdt::d9_usdt::D9USDTRef;
        //   use openbrush::contracts::psp22::psp22_external::PSP22;
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        #[ink_e2e::test]
        async fn check_liquidity(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let d9_liquidity: Balance = 10_000_000000000000;
            let usdt_liquidity: Balance = 10_000_00;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
                .expect("failed to instantiate usdt").account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 10);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None).await
                .expect("failed to instantiate market maker").account_id;

            //build approval message
            let caller = account_id(AccountKeyring::Alice);
            let check_liquidity_message = build_message::<MarketMakerRef>(amm_address.clone()).call(
                |market_maker| market_maker.check_new_liquidity(d9_liquidity, usdt_liquidity)
            );

            let response = client.call(&ink_e2e::alice(), check_liquidity_message, 0, None).await;
            // execute approval call
            assert!(response.is_ok());
            Ok(())
        }
        #[ink_e2e::test]
        async fn check_usdt_balance(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
                .expect("failed to instantiate usdt").account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None).await
                .expect("failed to instantiate market maker").account_id;

            //build approval message
            let caller = account_id(AccountKeyring::Alice);
            let check_usdt_balance_message = build_message::<MarketMakerRef>(
                amm_address.clone()
            ).call(|market_maker|
                market_maker.check_usdt_balance(caller, initial_supply.saturating_div(2000))
            );

            let response = client.call(
                &ink_e2e::alice(),
                check_usdt_balance_message,
                0,
                None
            ).await;
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
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
                .expect("failed to instantiate usdt").account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None).await
                .expect("failed to instantiate market maker").account_id;

            //build approval message
            let usdt_approved_amount = initial_supply.saturating_div(2000);
            let approval_message = build_message::<D9USDTRef>(usdt_address.clone()).call(|d9_usdt|
                d9_usdt.approve(
                    ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                    amm_address.clone(),
                    usdt_approved_amount
                )
            );
            // execute approval call
            let approval_response = client.call(&ink_e2e::alice(), approval_message, 0, None).await;
            assert!(approval_response.is_ok());

            //check allowance
            let caller = account_id(AccountKeyring::Alice);
            let check_usdt_allowance_message = build_message::<MarketMakerRef>(
                amm_address.clone()
            ).call(|market_maker|
                market_maker.check_usdt_allowance(caller, usdt_approved_amount.saturating_div(10))
            );

            let response = client.call(
                &ink_e2e::alice(),
                check_usdt_allowance_message,
                0,
                None
            ).await;
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
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
                .expect("failed to instantiate usdt").account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 100, 3);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None).await
                .expect("failed to instantiate market maker").account_id;

            //build approval message
            let usdt_approval_amount = 100_000_000_000_000;
            let approval_message = build_message::<D9USDTRef>(usdt_address.clone()).call(|d9_usdt|
                d9_usdt.approve(
                    ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                    amm_address.clone(),
                    usdt_approval_amount
                )
            );
            // execute approval call
            let approval_response = client.call(&ink_e2e::alice(), approval_message, 0, None).await;
            assert!(approval_response.is_ok());

            // add liquidity
            let usdt_liquidity_amount = usdt_approval_amount.saturating_div(20);
            let d9_liquidity_amount = usdt_liquidity_amount.saturating_div(10);
            let add_liquidity_message = build_message::<MarketMakerRef>(amm_address.clone()).call(
                |market_maker| market_maker.add_liquidity(usdt_liquidity_amount)
            );
            let add_liquidity_response = client.call(
                &ink_e2e::alice(),
                add_liquidity_message,
                d9_liquidity_amount,
                None
            ).await;

            assert!(add_liquidity_response.is_ok());
            Ok(())
        }
        // setup default contracts
    }
} //---LAST LINE OF IMPLEMENTATION OF THE INK! SMART CONTRACT---//
