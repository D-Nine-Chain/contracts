#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod market_maker {
    use super::*;
    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use sp_arithmetic::Perbill;
    use ink::selector_bytes;
    use ink::env::call::{ build_call, ExecutionInput, Selector };

    #[ink(storage)]
    pub struct MarketMaker {
        /// contract for usdt coin
        usdt_contract: AccountId,
        /// pool balances
        currency_balances: Mapping<Currency, Balance>,
        /// Perbill::from_rational(fee_numerator, fee_denominator)
        fee_numerator: u32,
        /// Perbill::from_rational(fee_numerator, fee_denominator)
        fee_denominator: u32,
        /// total fees collected
        fee_total: Balance,
        /// the invariant of the curve k = x * y
        curve_k: Balance,
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
    }

    impl MarketMaker {
        #[ink(constructor)]
        pub fn new(usdt_contract: AccountId, fee_numerator: u32, fee_denominator: u32) -> Self {
            let curve_k: Balance = (100 as Balance).saturating_mul(1_000_000_000_000);
            let liquidity_tolerance_percent: u32 = 8;
            Self {
                usdt_contract,
                currency_balances: Default::default(),
                fee_numerator,
                fee_denominator,
                fee_total: Default::default(),
                curve_k,
                liquidity_tolerance_percent,
                liquidity_providers: Default::default(),
                lp_tokens: Default::default(),
            }
        }

        /// get pool balances (d9, usdt)
        #[ink(message)]
        pub fn get_currency_reserves(&self) -> (Balance, Balance) {
            let d9_balance: Balance = self.currency_balances.get(Currency::D9).unwrap();
            let usdt_balance: Balance = self.currency_balances.get(Currency::USDT).unwrap();
            (d9_balance, usdt_balance)
        }

        /// add liquidity by adding tokens to the reserves
        #[ink(message, payable)]
        pub fn add_liquidity(&mut self, usdt_liquidity: Balance) -> Result<(), Error> {
            //get liquidity provider info
            let caller = self.env().caller();
            let liquidity_provider_result = self.liquidity_providers.get(&caller);
            let liquidity_provider = match liquidity_provider_result {
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

            // make sure new liquidity doesn't deviate price more than tolerance
            let liquidity_check = self.check_new_liquidity(d9_liquidity, usdt_liquidity);
            if liquidity_check.is_err() {
                return Err(liquidity_check.unwrap_err());
            }

            // Update reserves.
            let mut usdt_reserves = self.currency_balances.get(Currency::USDT).unwrap();
            let mut d9_reserves = self.currency_balances.get(Currency::D9).unwrap();

            usdt_reserves = usdt_reserves.saturating_add(usdt_liquidity);
            d9_reserves = d9_reserves.saturating_add(d9_liquidity);

            self.currency_balances.insert(Currency::USDT, &usdt_reserves);
            self.currency_balances.insert(Currency::D9, &d9_reserves);

            // Calculate liquidity token amount to mint.
            self.mint_lp_tokens(usdt_liquidity.clone(), liquidity_provider.clone());

            Ok(())
        }

        //   #[ink(message)]
        //   pub fn remove_liquidity(&mut self, usdt: Balance) -> Result<(), Error> {}

        /// ensure added liquidity will not deviate price more than tolerance
        #[ink(message)]
        pub fn check_new_liquidity(
            &self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance
        ) -> Result<(), Error> {
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();

            // Compute the ideal amount of d9_liquidity for the given usdt_liquidity using Perbill for precision
            let price_ratio = Perbill::from_rational(d9_reserves, usdt_reserves);
            let ideal_d9_for_provided_usdt = price_ratio.mul_floor(usdt_liquidity);

            // Determine the deviation allowed in absolute terms using Perbill
            let allowed_deviation_fraction = Perbill::from_percent(
                self.liquidity_tolerance_percent
            );
            let allowed_deviation_amount = allowed_deviation_fraction.mul_floor(
                ideal_d9_for_provided_usdt
            );

            // Calculate bounds for d9_liquidity using saturating arithmetic
            let min_d9 = ideal_d9_for_provided_usdt.saturating_sub(allowed_deviation_amount);
            let max_d9 = ideal_d9_for_provided_usdt.saturating_add(allowed_deviation_amount);

            // Check if the provided d9_liquidity is within bounds
            if d9_liquidity >= min_d9 && d9_liquidity <= max_d9 {
                Ok(())
            } else {
                Err(Error::InsufficientLiquidityProvided)
            }
        }

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

            let direction = Direction(Currency::USDT, Currency::D9);
            self.update_balances(direction, usdt, d9);

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
            self.update_balances(direction, d9, usdt);

            Ok((Currency::USDT, usdt))
        }

        /// mint lp tokens, credit provider account
        fn mint_lp_tokens(
            &mut self,
            new_usdt_liquidity: Balance,
            mut liquidity_provider: LiquidityProvider
        ) {
            let new_lp_tokens = self.calc_new_lp_tokens(new_usdt_liquidity);
            self.lp_tokens = self.lp_tokens.saturating_add(new_lp_tokens);
            liquidity_provider.lp_tokens =
                liquidity_provider.lp_tokens.saturating_add(new_lp_tokens);
            self.liquidity_providers.insert(liquidity_provider.account_id, &liquidity_provider);
        }

        /// calculate lp tokens based on usdt liquidity
        fn calc_new_lp_tokens(&mut self, usdt_liquidity: Balance) -> Balance {
            if self.lp_tokens == 0 {
                return 1_000_000;
            }
            let total_usdt = self.currency_balances.get(Currency::USDT).unwrap();

            let liquidity_ratio = Perbill::from_rational(usdt_liquidity as u32, total_usdt as u32);

            liquidity_ratio.mul_floor(self.lp_tokens)
        }

        /// remove lp tokens from system after liquidity is removed
        fn burn_lp_tokens(
            &mut self,
            amount: Balance,
            liquidity_provider_account: AccountId
        ) -> Result<(), Error> {
            // Get the liquidity provider's details; return an error if they don't exist.
            let get_liquidity_provider_result = self.liquidity_providers
                .get(&liquidity_provider_account)
                .ok_or(Error::LiquidityProviderNotFound);

            let mut liquidity_provider = get_liquidity_provider_result.unwrap();
            // Check if the liquidity provider and the contract have enough LP tokens to burn; if not, return an error.
            if liquidity_provider.lp_tokens < amount {
                return Err(Error::InsufficientLPTokens);
            }
            if self.lp_tokens < amount {
                return Err(Error::InsufficientContractLPTokens);
            }

            // Update the LP tokens for both the liquidity provider and the contract.
            liquidity_provider.lp_tokens = liquidity_provider.lp_tokens.saturating_sub(amount);
            self.lp_tokens = self.lp_tokens.saturating_sub(amount);

            // Update the liquidity provider's info in the storage.
            self.liquidity_providers.insert(liquidity_provider_account, &liquidity_provider);

            Ok(())
        }

        /// amount of currency B from A, if A => B
        fn calculate_exchange(
            &self,
            direction: &Direction,
            amount_0: Balance
        ) -> Result<Balance, Error> {
            //naming comes from Direction. e.g. direction.0 is the first currency in the pair

            // get currency balances
            let balance_0: Balance = self.currency_balances.get(direction.0).unwrap();
            let balance_1: Balance = self.currency_balances.get(direction.1).unwrap();

            // liquidity checks
            if balance_1 == 0 {
                return Err(Error::InsufficientLiquidity(direction.1));
            }
            if balance_0 < amount_0 {
                return Err(Error::InsufficientLiquidity(direction.0));
            }

            let new_balance_0: Balance = balance_0.saturating_add(amount_0);

            let new_balance_1: Balance = self.curve_k.saturating_div(new_balance_0);

            let amount_1: Balance = balance_1.saturating_sub(new_balance_1);

            Ok(amount_1)
        }

        fn update_balances(&mut self, direction: Direction, amount_0: Balance, amount_1: Balance) {
            // get currency balances
            let mut balance_0: Balance = self.currency_balances.get(direction.0).unwrap();
            let mut balance_1: Balance = self.currency_balances.get(direction.1).unwrap();

            //update
            balance_0 = balance_0.saturating_add(amount_0);
            balance_1 = balance_1.saturating_sub(amount_1);
            self.currency_balances.insert(direction.0, &balance_0);
            self.currency_balances.insert(direction.1, &balance_1);
        }

        /// get exchange fee
        fn calculate_fee(&self, amount: Balance) -> Result<Balance, Error> {
            let percent: Perbill = Perbill::from_rational(self.fee_numerator, self.fee_denominator);
            let fee: Balance = percent.mul_ceil(amount);
            if fee == 0 {
                return Err(Error::ConversionAmountTooLow);
            }
            Ok(fee)
        }

        /// get an account's usdt balance
        fn get_usdt_balance(&self, account_id: AccountId) -> Balance {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("balance_of"))).push_arg(
                        account_id
                    )
                )
                .returns::<Balance>()
                .invoke()
        }

        /// check if usdt balance is sufficient for swap
        fn check_usdt_balance(&self, account_id: AccountId, amount: Balance) -> Result<(), Error> {
            let usdt_balance = self.get_usdt_balance(account_id);

            if usdt_balance < amount {
                return Err(Error::USDTBalanceInsufficient);
            }
            Ok(())
        }

        fn check_usdt_allowance(&self, owner: AccountId, amount: Balance) -> Result<(), Error> {
            let allowance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("allowance")))
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

        fn send_usdt_to_user(&self, recipient: AccountId, amount: Balance) -> Result<(), Error> {
            let usdt_balance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("transfer")))
                        .push_arg(recipient)
                        .push_arg(amount)
                )
                .returns::<Balance>()
                .invoke();
            if usdt_balance < amount {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::USDT));
            }
            Ok(())
        }

        fn receive_usdt_from_user(&self, sender: AccountId, amount: Balance) -> Result<(), Error> {
            let _ = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("transfer_from")))
                        .push_arg(sender)
                        .push_arg(self.env().account_id())
                        .push_arg(amount) //note maybe data. check.
                )
                .returns::<Balance>()
                .invoke();
            Ok(())
        }
    }
} //---LAST LINE OF IMPLEMENTATION OF THE INK! SMART CONTRACT---//
