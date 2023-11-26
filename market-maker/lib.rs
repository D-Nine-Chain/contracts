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
        fee_percent: u32,
        /// total fees collected
        fee_total: Balance,
        ///represents numerator of a percent
        liquidity_tolerance_percent: u32,
        /// providers of contract liquidity
        liquidity_providers: Mapping<AccountId, Balance>,
        /// total number of liquidity pool tokens
        total_lp_tokens: Balance,
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
        D9orUSDTProvidedLiquidityAtZero,
        ConversionAmountTooLow,
        CouldntTransferUSDTFromUser,
        InsufficientLiquidity(Currency),
        InsufficientAllowance,
        MarketMakerHasInsufficientFunds(Currency),
        InsufficientLiquidityProvided,
        USDTBalanceInsufficient,
        LiquidityProviderNotFound,
        LiquidityAddedBeyondTolerance,
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
            fee_percent: u32,
            liquidity_tolerance_percent: u32
        ) -> Self {
            assert!(
                0 < liquidity_tolerance_percent && liquidity_tolerance_percent <= 100,
                "tolerance must be 0 < x <= 100"
            );
            Self {
                usdt_contract,
                fee_percent,
                fee_total: Default::default(),
                liquidity_tolerance_percent,
                liquidity_providers: Default::default(),
                total_lp_tokens: Default::default(),
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
        pub fn get_liquidity_provider(&self, account_id: AccountId) -> Option<Balance> {
            self.liquidity_providers.get(&account_id)
        }
        /// add liquidity by adding tokens to the reserves
        #[ink(message, payable)]
        pub fn add_liquidity(&mut self, usdt_liquidity: Balance) -> Result<(), Error> {
            let caller = self.env().caller();

            let d9_liquidity = self.env().transferred_value();
            // greeater than zero checks
            if usdt_liquidity == 0 || d9_liquidity == 0 {
                return Err(Error::D9orUSDTProvidedLiquidityAtZero);
            }
            let validity_check = self.usdt_validity_check(caller, usdt_liquidity);
            if let Err(e) = validity_check {
                return Err(e);
            }

            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();
            if usdt_reserves != 0 && d9_reserves != 0 {
                let liquidity_check = self.check_new_liquidity(usdt_liquidity, d9_liquidity);
                if let Err(e) = liquidity_check {
                    return Err(e);
                }
            }

            // receive usdt from user
            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt_liquidity);
            if receive_usdt_result.is_err() {
                return Err(Error::CouldntTransferUSDTFromUser);
            }

            self.mint_lp_tokens(caller, d9_liquidity, usdt_liquidity);

            Ok(())
        }

        #[ink(message)]
        pub fn remove_liquidity(&mut self) -> Result<(), Error> {
            let caller = self.env().caller();
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();

            let lp_tokens = self.liquidity_providers.get(&caller).unwrap();
            if lp_tokens == 0 {
                return Err(Error::LiquidityProviderNotFound);
            }

            // Calculate  contribution
            let liquidity_percent = self.calculate_lp_percent(lp_tokens);
            let d9_liquidity = liquidity_percent.saturating_mul_int(d9_reserves);
            let usdt_liquidity = liquidity_percent.saturating_mul_int(usdt_reserves);

            // Transfer payouts
            let transfer_result = self.env().transfer(caller, d9_liquidity.to_num::<Balance>());
            if transfer_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::D9));
            }
            let send_usdt_result = self.send_usdt_to_user(
                caller,
                usdt_liquidity.to_num::<Balance>()
            );
            if send_usdt_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::USDT));
            }

            // update liquidity provider
            self.total_lp_tokens = self.total_lp_tokens.saturating_sub(lp_tokens);
            self.liquidity_providers.remove(&caller);

            Ok(())
        }

        fn calculate_lp_percent(&self, lp_tokens: Balance) -> FixedBalance {
            let percent_provided = FixedBalance::from_num(lp_tokens).checked_div(
                FixedBalance::from_num(self.total_lp_tokens)
            );
            if percent_provided.is_none() {
                return FixedBalance::from_num(0);
            }
            percent_provided.unwrap()
        }

        #[ink(message)]
        pub fn check_new_liquidity(
            &self,
            usdt_liquidity: Balance,
            d9_liquidity: Balance
        ) -> Result<(), Error> {
            let usdt_mult_factor = 10_000_000_000;
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();

            let current_price_option: Option<FixedBalance> = FixedBalance::from_num(
                usdt_reserves.saturating_mul(usdt_mult_factor)
            ).checked_div(FixedBalance::from_num(d9_reserves));
            let current_price = match current_price_option {
                Some(price) => price,
                None => {
                    return Err(Error::DivisionByZero);
                }
            };
            let new_price_option: Option<FixedBalance> = FixedBalance::from_num(
                usdt_liquidity.saturating_mul(usdt_mult_factor)
            ).checked_div(FixedBalance::from_num(d9_liquidity));

            let new_price = match new_price_option {
                Some(price) => price,
                None => {
                    return Err(Error::DivisionByZero);
                }
            };

            let price_difference = {
                if current_price > new_price {
                    current_price.saturating_sub(new_price)
                } else {
                    new_price.saturating_sub(current_price)
                }
            };

            let difference_percent = price_difference.checked_div(current_price).unwrap();
            let threshold = FixedBalance::from_num(self.liquidity_tolerance_percent)
                .checked_div(FixedBalance::from_num(100))
                .unwrap();
            if threshold < difference_percent {
                return Err(Error::LiquidityAddedBeyondTolerance);
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
                return Err(Error::CouldntTransferUSDTFromUser);
            }

            //prepare d9 to send
            let d9_calc_result = self.calculate_exchange(
                Direction(Currency::USDT, Currency::D9),
                usdt
            );
            if let Err(e) = d9_calc_result {
                return Err(e);
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
            let usdt_calc_result = self.calculate_exchange(direction, d9);
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
            provider_id: AccountId,
            new_d9_liquidity: Balance,
            new_usdt_liquidity: Balance
        ) {
            let provider_current_lp = self.liquidity_providers
                .get(&provider_id)
                .unwrap_or_default();

            let new_lp_tokens = self.calc_new_lp_tokens(new_d9_liquidity, new_usdt_liquidity);
            //add tokens to lp provider and contract total
            self.total_lp_tokens = self.total_lp_tokens.saturating_add(new_lp_tokens);

            let updated_provider_lp = provider_current_lp.saturating_add(new_lp_tokens);

            self.liquidity_providers.insert(provider_id, &updated_provider_lp);
        }

        /// calculate lp tokens based on usdt liquidity
        #[ink(message)]
        pub fn calc_new_lp_tokens(
            &mut self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance
        ) -> Balance {
            // Initialize LP tokens if the pool is empty
            if self.total_lp_tokens == 0 {
                return 1_000_000;
            }
            // Get current reserves
            let (d9_reserve, usdt_reserve) = self.get_currency_reserves();
            let current_reserve_total = d9_reserve.saturating_add(usdt_reserve);

            let new_liquidity_total = d9_liquidity.saturating_add(usdt_liquidity);
            let new_liquidity_ratio = FixedBalance::from_num(new_liquidity_total)
                .checked_div(FixedBalance::from_num(current_reserve_total))
                .unwrap_or(FixedBalance::from_num(0));

            let new_lp_tokens = new_liquidity_ratio.saturating_mul_int(self.total_lp_tokens);

            new_lp_tokens.to_num::<Balance>()
        }

        fn usdt_validity_check(&self, caller: AccountId, amount: Balance) -> Result<(), Error> {
            // does sender have sufficient usdt
            let usdt_balance_check_result = self.check_usdt_balance(caller, amount);
            if usdt_balance_check_result.is_err() {
                return Err(usdt_balance_check_result.unwrap_err());
            }

            // did sender provider sufficient allowance permission
            let usdt_allowance_check = self.check_usdt_allowance(caller, amount);
            if usdt_allowance_check.is_err() {
                return Err(usdt_allowance_check.unwrap_err());
            }
            Ok(())
        }
        /// amount of currency B from A, if A => B
        #[ink(message)]
        pub fn calculate_exchange(
            &self,
            direction: Direction,
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
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::test::default_accounts;
        use substrate_fixed::{ FixedU128, types::extra::U6 };
        type FixedBalance = FixedU128<U6>;
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
            let usdt_mult_factor = 10_000_000_000;
            let d9_liquidity: Balance = 1_100_000_000_000_000;
            let usdt_liquidity: Balance = 150_00;

            let (d9_reserves, usdt_reserves): (Balance, Balance) = (1_000_000_000_000_000, 1000_00);
            let current_price_option: Option<FixedBalance> = FixedBalance::from_num(
                usdt_reserves.saturating_mul(usdt_mult_factor)
            ).checked_div(FixedBalance::from_num(d9_reserves));
            let current_price = current_price_option.unwrap();
            println!("current price is: {}", current_price);
            let new_price_option: Option<FixedBalance> = FixedBalance::from_num(
                usdt_liquidity.saturating_mul(usdt_mult_factor)
            ).checked_div(FixedBalance::from_num(d9_liquidity));

            let new_price = new_price_option.unwrap();
            println!("new price is: {}", new_price);
            let price_difference = {
                if current_price > new_price {
                    current_price.saturating_sub(new_price)
                } else {
                    new_price.saturating_sub(current_price)
                }
            };
            let difference_percent = price_difference.checked_div(current_price).unwrap();
            let threshold = FixedBalance::from_num(10)
                .checked_div(FixedBalance::from_num(100))
                .unwrap();
            println!("price difference percent: {}", difference_percent);
            assert!(difference_percent < threshold)
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
