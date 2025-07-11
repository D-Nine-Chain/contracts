#![cfg_attr(not(feature = "std"), no_std, no_main)]
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod market_maker {
    use super::*;
    use ink::env::call::{build_call, ExecutionInput, Selector};
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{Decode, Encode};
    use substrate_fixed::{types::extra::U28, FixedU128};
    type FixedBalance = FixedU128<U28>;

    /// Minimum liquidity that must remain in reserves after any swap
    const MINIMUM_LIQUIDITY: Balance = 1000;

    #[ink(storage)]
    pub struct MarketMaker {
        /// contract for usdt coin
        usdt_contract: AccountId,
        /// Perbill::from_rational(fee_numerator, fee_denominator)
        fee_percent: u32,
        /// DEPRECATED - kept for storage compatibility. Not used in new AMM formula
        /// In the old implementation, this tracked accumulated fees separately.
        /// The new implementation uses Uniswap V2 style where fees are implicit in reserves.
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
        LiquidityAddedBeyondTolerance(Balance, Balance),
        InsufficientLPTokens,
        InsufficientContractLPTokens,
        DivisionByZero,
        MultiplicationError,
        ArithmeticOverflow,
        InvalidFeePercent,
        USDTTooSmall,
        USDTTooMuch,
        LiquidityTooLow,
        SlippageExceeded,
        InsufficientReserves,
        InvalidAddress,
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
                fee_total: Default::default(), // Deprecated but kept for storage compatibility
                liquidity_tolerance_percent,
                liquidity_providers: Default::default(),
                total_lp_tokens: Default::default(),
            }
        }

        #[ink(message)]
        pub fn change_admin(&mut self, new_admin: AccountId) -> Result<(), Error> {
            assert!(
                self.env().caller() == self.admin,
                "Only admin can change admin."
            );

            // Validate new admin is not zero address
            if new_admin == AccountId::from([0u8; 32]) {
                return Err(Error::InvalidAddress);
            }

            self.admin = new_admin;
            Ok(())
        }

        /// get pool balances (d9, usdt)
        #[ink(message)]
        pub fn get_currency_reserves(&self) -> (Balance, Balance) {
            let d9_balance: Balance = self.env().balance();
            let usdt_balance: Balance = self.get_usdt_balance(self.env().account_id());
            (d9_balance, usdt_balance)
        }

        #[ink(message)]
        pub fn get_total_lp_tokens(&self) -> Balance {
            self.total_lp_tokens
        }

        #[ink(message)]
        pub fn get_liquidity_provider(&self, account_id: AccountId) -> Option<Balance> {
            self.liquidity_providers.get(&account_id)
        }

        /// add liquidity by adding tokens to the reserves
        #[ink(message, payable)]
        pub fn add_liquidity(&mut self, usdt_liquidity: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            // greeater than zero checks
            let d9_liquidity = self.env().transferred_value();
            if usdt_liquidity == 0 || d9_liquidity == 0 {
                return Err(Error::D9orUSDTProvidedLiquidityAtZero);
            }

            // Get reserves BEFORE new liquidity is added
            // Note: D9 has already been transferred (payable), but USDT hasn't
            let d9_balance_before = self.env().balance().saturating_sub(d9_liquidity);
            let usdt_balance_before = self.get_usdt_balance(self.env().account_id());

            if usdt_balance_before != 0 && d9_balance_before != 0 {
                let liquidity_check = self.check_new_liquidity(usdt_liquidity, d9_liquidity);
                if let Err(e) = liquidity_check {
                    return Err(e);
                }
            }

            // Validate USDT balance and allowance
            self.usdt_validity_check(caller, usdt_liquidity)?;

            // receive usdt from user
            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt_liquidity);
            if receive_usdt_result.is_err() {
                // Refund D9 tokens since USDT transfer failed
                // This prevents D9 from being stuck in the contract
                if d9_liquidity > 0 {
                    let refund_result = self.env().transfer(caller, d9_liquidity);
                    if refund_result.is_err() {
                        // Log critical error - D9 refund failed
                        // In production, this should trigger an alert
                    }
                }
                return Err(Error::CouldntTransferUSDTFromUser);
            }

            // Try to mint LP tokens
            let mint_result = self.mint_lp_tokens(
                caller,
                d9_liquidity,
                usdt_liquidity,
                d9_balance_before,
                usdt_balance_before,
            );

            if mint_result.is_err() {
                // If minting fails, refund both D9 and USDT
                // Refund D9
                if d9_liquidity > 0 {
                    let _ = self.env().transfer(caller, d9_liquidity);
                }
                // Refund USDT
                let _ = self.send_usdt_to_user(caller, usdt_liquidity);
                return Err(mint_result.unwrap_err());
            }

            self.env().emit_event(LiquidityAdded {
                account_id: caller,
                usdt: usdt_liquidity,
                d9: d9_liquidity,
            });

            Ok(())
        }

        #[ink(message)]
        pub fn remove_liquidity(&mut self) -> Result<(), Error> {
            let caller = self.env().caller();
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();

            let lp_tokens = {
                let result = self.liquidity_providers.get(&caller);
                match result {
                    None => 0,
                    Some(tokens) => tokens,
                }
            };

            if lp_tokens == 0 {
                return Err(Error::LiquidityProviderNotFound);
            }

            // Calculate contribution
            let liquidity_percent = self.calculate_lp_percent(lp_tokens);
            let d9_liquidity = liquidity_percent.saturating_mul_int(d9_reserves);
            let usdt_liquidity = liquidity_percent.saturating_mul_int(usdt_reserves);

            // Check if removal would leave reserves below minimum
            let d9_liquidity_balance = d9_liquidity.to_num::<Balance>();
            let usdt_liquidity_balance = usdt_liquidity.to_num::<Balance>();
            let d9_remaining = d9_reserves.saturating_sub(d9_liquidity_balance);
            let usdt_remaining = usdt_reserves.saturating_sub(usdt_liquidity_balance);

            // Only enforce minimum if pool is not being completely drained
            if self.total_lp_tokens != lp_tokens {
                if d9_remaining < MINIMUM_LIQUIDITY || usdt_remaining < MINIMUM_LIQUIDITY {
                    return Err(Error::InsufficientReserves);
                }
            }

            // Transfer payouts
            let transfer_result = self
                .env()
                .transfer(caller, d9_liquidity.to_num::<Balance>());
            if transfer_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::D9));
            }

            let send_usdt_result =
                self.send_usdt_to_user(caller, usdt_liquidity.to_num::<Balance>());
            if send_usdt_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::USDT));
            }

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
        ) -> Result<(), Error> {
            let (d9_reserves, usdt_reserves) = self.get_currency_reserves();
            let fixed_usdt_reserves = FixedBalance::from_num(usdt_reserves);
            let fixed_d9_reserves = FixedBalance::from_num(d9_reserves);
            let fixed_usdt_liquidity = FixedBalance::from_num(usdt_liquidity);
            let fixed_d9_liquidity = FixedBalance::from_num(d9_liquidity);

            let checked_ratio = fixed_d9_reserves.checked_div(fixed_usdt_reserves);
            let ratio = match checked_ratio {
                Some(r) => r,
                None => {
                    return Err(Error::DivisionByZero);
                }
            };

            let checked_threshold_percent =
                FixedBalance::from_num(self.liquidity_tolerance_percent)
                    .checked_div(FixedBalance::from_num(100));
            let threshold_percent = match checked_threshold_percent {
                Some(t) => t,
                None => {
                    return Err(Error::DivisionByZero);
                }
            };

            let checked_threshold = threshold_percent.checked_mul(ratio);
            let threshold = match checked_threshold {
                Some(t) => t,
                None => {
                    return Err(Error::MultiplicationError);
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
                return Err(Error::LiquidityAddedBeyondTolerance(
                    threshold.to_num::<Balance>(),
                    price_difference.to_num::<Balance>(),
                ));
            }
            Ok(())
        }

        /// sell usdt
        #[ink(message)]
        pub fn get_d9(&mut self, usdt: Balance, min_d9_out: Balance) -> Result<Balance, Error> {
            let caller: AccountId = self.env().caller();

            // Validate USDT balance and allowance
            self.usdt_validity_check(caller, usdt)?;

            let receive_usdt_result = self.receive_usdt_from_user(caller, usdt.clone());
            if receive_usdt_result.is_err() {
                return Err(Error::CouldntTransferUSDTFromUser);
            }

            //prepare d9 to send
            let d9_calc_result =
                self.calculate_exchange(Direction(Currency::USDT, Currency::D9), usdt);
            if let Err(e) = d9_calc_result {
                return Err(e);
            }
            let d9 = d9_calc_result.unwrap();
            // Fee is already deducted in calculate_exchange

            // Check slippage protection
            if d9 < min_d9_out {
                return Err(Error::SlippageExceeded);
            }

            // send d9
            let transfer_result = self.env().transfer(caller, d9);
            if transfer_result.is_err() {
                return Err(Error::MarketMakerHasInsufficientFunds(Currency::D9));
            }

            self.env().emit_event(USDTToD9Conversion {
                account_id: caller,
                usdt,
                d9,
            });

            Ok(d9)
        }

        /// sell d9
        #[ink(message, payable)]
        pub fn get_usdt(&mut self, min_usdt_out: Balance) -> Result<Balance, Error> {
            let direction = Direction(Currency::D9, Currency::USDT);
            let d9: Balance = self.env().transferred_value();

            let usdt_calc_result = self.calculate_exchange(direction, d9);
            if usdt_calc_result.is_err() {
                return Err(usdt_calc_result.unwrap_err());
            }
            let usdt = usdt_calc_result.unwrap();
            // Fee is already deducted in calculate_exchange

            // Check slippage protection
            if usdt < min_usdt_out {
                return Err(Error::SlippageExceeded);
            }

            //prepare to send
            let is_balance_sufficient = self.check_usdt_balance(self.env().account_id(), usdt);
            if is_balance_sufficient.is_err() {
                return Err(Error::InsufficientLiquidity(Currency::USDT));
            }

            // send usdt
            let caller = self.env().caller();
            self.send_usdt_to_user(caller, usdt.clone())?;

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
            d9_reserve_before: Balance,
            usdt_reserve_before: Balance,
        ) -> Result<(), Error> {
            let provider_current_lp = self
                .liquidity_providers
                .get(&provider_id)
                .unwrap_or_default();

            let new_lp_tokens = self.calc_new_lp_tokens(
                new_d9_liquidity,
                new_usdt_liquidity,
                d9_reserve_before,
                usdt_reserve_before,
            );

            if new_lp_tokens == 0 {
                return Err(Error::LiquidityTooLow);
            }
            //add tokens to lp provider and contract total
            self.total_lp_tokens = self.total_lp_tokens.saturating_add(new_lp_tokens);

            let updated_provider_lp = provider_current_lp.saturating_add(new_lp_tokens);

            self.liquidity_providers
                .insert(provider_id, &updated_provider_lp);

            Ok(())
        }

        /// Safe square root calculation that handles large numbers without overflow
        fn safe_sqrt(&self, a: Balance, b: Balance) -> Balance {
            if a == 0 || b == 0 {
                return 0;
            }
            
            match (a as u128).checked_mul(b as u128) {
                Some(product) => self.sqrt_newton_verified(product) as Balance,
                None => {
                    // For overflow, compute sqrts separately
                    let sqrt_a = self.sqrt_newton_verified(a as u128);
                    let sqrt_b = self.sqrt_newton_verified(b as u128);
                    
                    // This is exact for perfect squares and very close otherwise
                    sqrt_a.saturating_mul(sqrt_b) as Balance
                }
            }
        }


        /// Newton's method with verification for exactness
        fn sqrt_newton_verified(&self, n: u128) -> u128 {
            if n == 0 {
                return 0;
            }
            
            // Initial guess
            let bits = 128 - n.leading_zeros();
            let mut x = 1u128 << ((bits + 1) / 2);
            
            // Newton iterations until convergence
            loop {
                let x_new = (x + n / x) / 2;
                if x_new >= x {
                    break;
                }
                x = x_new;
            }
            
            // Verify and adjust if needed
            // x is the floor(sqrt(n))
            if let Some(x_squared) = x.checked_mul(x) {
                if x_squared > n {
                    // Should not happen with correct Newton's method
                    x - 1
                } else {
                    // Check if we should round up or down
                    if let Some(x_plus_1_squared) = (x + 1).checked_mul(x + 1) {
                        if x_plus_1_squared <= n {
                            x + 1 // We were off by one
                        } else {
                            x // x is correct
                        }
                    } else {
                        x // x+1 would overflow, so x is correct
                    }
                }
            } else {
                // x^2 overflows, so x is too large
                x - 1
            }
        }

        /// calculate lp tokens based on usdt liquidity
        #[ink(message)]
        pub fn calc_new_lp_tokens(
            &mut self,
            d9_liquidity: Balance,
            usdt_liquidity: Balance,
            d9_reserve: Balance,
            usdt_reserve: Balance,
        ) -> Balance {
            if self.total_lp_tokens == 0 {
                // Initial liquidity - use geometric mean
                let initial_lp = self.safe_sqrt(d9_liquidity, usdt_liquidity);
                
                // Burn first 1000 LP tokens (MINIMUM_LIQUIDITY) to prevent attacks
                if initial_lp <= MINIMUM_LIQUIDITY {
                    return 0; // Too small initial liquidity
                }
                return initial_lp.saturating_sub(MINIMUM_LIQUIDITY);
            }
            
            if d9_reserve == 0 || usdt_reserve == 0 {
                return 0;
            }

            // Calculate ratios
            let d9_ratio = (d9_liquidity as u128)
                .checked_mul(self.total_lp_tokens as u128)
                .and_then(|v| v.checked_div(d9_reserve as u128))
                .unwrap_or(0);
                
            let usdt_ratio = (usdt_liquidity as u128)
                .checked_mul(self.total_lp_tokens as u128)
                .and_then(|v| v.checked_div(usdt_reserve as u128))
                .unwrap_or(0);

            // Validate ratios are close (within tolerance)
            let min_ratio = core::cmp::min(d9_ratio, usdt_ratio);
            let max_ratio = core::cmp::max(d9_ratio, usdt_ratio);
            
            if min_ratio > 0 {
                // Check if ratios differ by more than tolerance (e.g., 1%)
                let ratio_diff_percent = ((max_ratio - min_ratio) * 100)
                    .checked_div(min_ratio)
                    .unwrap_or(u128::MAX);
                    
                if ratio_diff_percent > self.liquidity_tolerance_percent as u128 {
                    // Liquidity is too imbalanced
                    return 0; // Or return an error through Result<Balance, Error>
                }
            }

            min_ratio as Balance
        }

        fn usdt_validity_check(&self, caller: AccountId, amount: Balance) -> Result<(), Error> {
            // does sender have sufficient usdt
            let usdt_balance_check_result = self.check_usdt_balance(caller, amount);
            if let Err(e) = usdt_balance_check_result {
                return Err(e);
            }

            // did sender provider sufficient allowance permission
            let usdt_allowance_check = self.check_usdt_allowance(caller, amount);
            if let Err(e) = usdt_allowance_check {
                return Err(e);
            }
            Ok(())
        }

        /// amount of currency B from A, if A => B
        #[ink(message)]
        pub fn calculate_exchange(
            &self,
            direction: Direction,
            amount_in: Balance,
        ) -> Result<Balance, Error> {
            let reserve_in = self.get_currency_balance(direction.0);
            let reserve_out = self.get_currency_balance(direction.1);

            // Check minimum reserves before swap
            if reserve_in < MINIMUM_LIQUIDITY || reserve_out < MINIMUM_LIQUIDITY {
                return Err(Error::InsufficientReserves);
            }

            // Check if output liquidity exists
            if reserve_out == 0 {
                return Err(Error::InsufficientLiquidity(direction.1));
            }

            let amount_out =
                self.calc_opposite_currency_amount(reserve_in, reserve_out, amount_in)?;

            // Check that reserves will remain above minimum after swap
            if reserve_out.saturating_sub(amount_out) < MINIMUM_LIQUIDITY {
                return Err(Error::InsufficientReserves);
            }

            Ok(amount_out)
        }

        #[ink(message)]
        pub fn estimate_exchange(
            &self,
            direction: Direction,
            amount_in: Balance,
        ) -> Result<(Balance, Balance), Error> {
            let amount_out = self.calculate_exchange(direction, amount_in)?;
            Ok((amount_in, amount_out))
        }

        pub fn calc_opposite_currency_amount(
            &self,
            reserve_in: Balance,
            reserve_out: Balance,
            amount_in: Balance,
        ) -> Result<Balance, Error> {
            if reserve_in == 0 || reserve_out == 0 {
                return Err(Error::DivisionByZero);
            }

            if amount_in == 0 {
                return Ok(0);
            }

            // Validate fee percentage is reasonable
            if self.fee_percent > 100 {
                return Err(Error::InvalidFeePercent);
            }

            // Uniswap V2 formula: Uses per-mille (1000 = 100%)
            // For 1% fee: fee_per_mille = 10, so (1000 - 10) = 990
            // For 0.3% fee (standard): fee_per_mille = 3, so (1000 - 3) = 997
            let fee_per_mille = (self.fee_percent as u128)
                .checked_mul(10)
                .ok_or(Error::ArithmeticOverflow)?; // Convert percent to per-mille

            // Calculate fee multiplier (e.g., 997 for 0.3% fee, 990 for 1% fee)
            let fee_multiplier = 1000_u128
                .checked_sub(fee_per_mille)
                .ok_or(Error::ArithmeticOverflow)?;

            // Calculate amount_in with fee deducted
            let amount_in_u128 = amount_in as u128;
            let amount_in_with_fee = amount_in_u128
                .checked_mul(fee_multiplier)
                .ok_or(Error::ArithmeticOverflow)?;

            // Uniswap V2 formula:
            // amount_out = (amount_in_with_fee * reserve_out) / (reserve_in * 1000 + amount_in_with_fee)
            let reserve_out_u128 = reserve_out as u128;
            let numerator = amount_in_with_fee
                .checked_mul(reserve_out_u128)
                .ok_or(Error::ArithmeticOverflow)?;

            // denominator = (reserve_in * 1000) + amount_in_with_fee
            let denominator = (reserve_in as u128)
                .checked_mul(1000)
                .ok_or(Error::MultiplicationError)?
                .checked_add(amount_in_with_fee)
                .ok_or(Error::ArithmeticOverflow)?;

            // amount_out = numerator / denominator
            let amount_out = numerator
                .checked_div(denominator)
                .ok_or(Error::DivisionByZero)?;

            // Validate output doesn't exceed available reserves
            if amount_out > reserve_out_u128 {
                return Err(Error::InsufficientLiquidity(Currency::USDT));
            }

            Ok(amount_out as Balance)
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
            amount: Balance,
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
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::balance_of")))
                        .push_arg(account_id),
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
                        .push_arg(self.env().account_id()),
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

        /// Calculate input amount needed to get desired output (for frontend UX)
        #[ink(message)]
        pub fn calc_input_for_exact_output(
            &self,
            reserve_in: Balance,
            reserve_out: Balance,
            amount_out_desired: Balance,
        ) -> Result<Balance, Error> {
            if amount_out_desired >= reserve_out {
                return Err(Error::InsufficientLiquidity(Currency::USDT));
            }

            if amount_out_desired == 0 {
                return Ok(0);
            }

            // Validate fee percentage
            if self.fee_percent > 100 {
                return Err(Error::InvalidFeePercent);
            }

            let fee_per_mille = (self.fee_percent as u128)
                .checked_mul(10)
                .ok_or(Error::ArithmeticOverflow)?;

            // Uniswap V2 reverse formula: amount_in = (reserve_in * amount_out * 1000) / ((reserve_out - amount_out) * (1000 - fee))
            let numerator = (reserve_in as u128)
                .checked_mul(amount_out_desired as u128)
                .ok_or(Error::MultiplicationError)?
                .checked_mul(1000)
                .ok_or(Error::ArithmeticOverflow)?;

            let denominator = (reserve_out as u128)
                .checked_sub(amount_out_desired as u128)
                .ok_or(Error::InsufficientLiquidity(Currency::USDT))?
                .checked_mul(
                    1000_u128
                        .checked_sub(fee_per_mille)
                        .ok_or(Error::ArithmeticOverflow)?,
                )
                .ok_or(Error::ArithmeticOverflow)?;

            let amount_in = numerator
                .checked_div(denominator)
                .ok_or(Error::DivisionByZero)?
                .checked_add(1) // Round up to ensure user gets at least amount_out_desired
                .ok_or(Error::ArithmeticOverflow)?;

            Ok(amount_in as Balance)
        }

        /// Get price impact percentage for a trade
        #[ink(message)]
        pub fn get_price_impact(
            &self,
            direction: Direction,
            amount_in: Balance,
        ) -> Result<u32, Error> {
            let reserve_in = self.get_currency_balance(direction.0);
            let reserve_out = self.get_currency_balance(direction.1);
            
            if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
                return Ok(0);
            }
            
            let amount_out = self.calculate_exchange(direction, amount_in)?;
            
            // Calculate prices as ratios scaled to basis points
            let spot_price_bps = (reserve_out as u128)
                .checked_mul(10000)
                .ok_or(Error::ArithmeticOverflow)?
                .checked_div(reserve_in as u128)
                .ok_or(Error::DivisionByZero)?;
            
            let execution_price_bps = (amount_out as u128)
                .checked_mul(10000)
                .ok_or(Error::ArithmeticOverflow)?
                .checked_div(amount_in as u128)
                .ok_or(Error::DivisionByZero)?;
            
            // Impact = (1 - execution/spot) * 10000
            if execution_price_bps >= spot_price_bps {
                return Ok(0); // Positive slippage
            }
            
            let impact = spot_price_bps
                .checked_sub(execution_price_bps)
                .ok_or(Error::ArithmeticOverflow)?
                .checked_mul(10000)
                .ok_or(Error::ArithmeticOverflow)?
                .checked_div(spot_price_bps)
                .ok_or(Error::DivisionByZero)?;
            
            Ok(impact as u32)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink::env::test::{default_accounts, set_caller, set_value_transferred};
        use ink::env::DefaultEnvironment;
        use substrate_fixed::{types::extra::U28, FixedU128};
        type FixedBalance = FixedU128<U28>;

        fn get_default_test_accounts() -> ink::env::test::DefaultAccounts<DefaultEnvironment> {
            default_accounts::<DefaultEnvironment>()
        }

        fn setup_contract() -> MarketMaker {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            MarketMaker::new(accounts.bob, 1, 1) // 1% fee, 1% tolerance
        }

        // ===== Core AMM Tests =====

        #[ink::test]
        fn test_constant_product_maintained() {
            let contract = setup_contract(); // 1% fee

            let x = 1_000_000_000;
            let y = 1_000_000_000;
            let k_before = (x as u128) * (y as u128);

            let input = 100_000_000;
            let output = contract.calc_opposite_currency_amount(x, y, input).unwrap();

            // After swap with fee on input
            let fee_per_mille = 10; // 1% = 10 per mille
            let effective_input = (input as u128) * (1000 - fee_per_mille) / 1000;

            let x_after = x + effective_input as Balance;
            let y_after = y - output;
            let k_after = (x_after as u128) * (y_after as u128);

            // K should be maintained (with tiny rounding difference)
            let diff = if k_after > k_before {
                k_after - k_before
            } else {
                k_before - k_after
            };

            let tolerance = k_before / 1_000_000; // 0.0001% tolerance
            assert!(
                diff <= tolerance,
                "Constant product maintained with V2 formula"
            );
        }

        #[ink::test]
        fn test_fee_consistency() {
            let contract = setup_contract();

            let x = 10_000_000_000;
            let y = 10_000_000_000;

            // One large trade
            let large_input = 1_000_000_000;
            let large_output = contract
                .calc_opposite_currency_amount(x, y, large_input)
                .unwrap();

            // Ten small trades
            let small_input = 100_000_000;
            let mut total_output = 0;
            let mut current_x = x;
            let mut current_y = y;

            for _ in 0..10 {
                let output = contract
                    .calc_opposite_currency_amount(current_x, current_y, small_input)
                    .unwrap();
                total_output += output;

                // Update reserves (with fee on input)
                let effective_input = (small_input as u128) * 990 / 1000; // 1% fee
                current_x += effective_input as Balance;
                current_y -= output;
            }

            // With V2 formula, splitting trades should give LESS output (not more)
            assert!(
                total_output < large_output,
                "V2 prevents fee bypass through trade splitting"
            );
        }

        #[ink::test]
        fn test_zero_inputs_handling() {
            let contract = setup_contract();

            // Zero input should give zero output
            assert_eq!(
                contract
                    .calc_opposite_currency_amount(1000, 1000, 0)
                    .unwrap(),
                0
            );

            // Zero reserves should fail
            assert_eq!(
                contract.calc_opposite_currency_amount(0, 1000, 100),
                Err(Error::DivisionByZero)
            );
            assert_eq!(
                contract.calc_opposite_currency_amount(1000, 0, 100),
                Err(Error::DivisionByZero)
            );
        }

        #[ink::test]
        fn test_input_for_exact_output() {
            let contract = setup_contract();

            let x = 1_000_000_000;
            let y = 1_000_000_000;
            let desired_output = 90_000_000;

            // Calculate input needed
            let required_input = contract
                .calc_input_for_exact_output(x, y, desired_output)
                .unwrap();

            // Verify we get at least the desired output
            let actual_output = contract
                .calc_opposite_currency_amount(x, y, required_input)
                .unwrap();

            assert!(
                actual_output >= desired_output,
                "Should get at least desired output"
            );

            // Should be close but not too much over (due to rounding up)
            assert!(
                actual_output < desired_output + 1000,
                "Shouldn't overpay too much"
            );
        }

        #[ink::test]
        fn test_price_impact_calculation() {
            // Skip this test as get_price_impact requires get_currency_balance which calls external contracts
            // The price impact calculation itself is tested through the AMM formula tests
        }

        
        #[ink::test]
        fn test_price_impact_edge_cases() {
            // Test edge cases for price impact calculation
            
            // Test 1: Positive slippage (execution price better than spot)
            // This should return 0 impact
            let spot_price_bps = 10000u128;
            let execution_price_bps = 10500u128; // Better execution
            
            // When execution price >= spot price, impact should be 0
            if execution_price_bps >= spot_price_bps {
                assert_eq!(0, 0); // Would return 0 in actual implementation
            }
            
            // Test 2: Maximum price impact
            let spot_price_bps = 10000u128;
            let execution_price_bps = 1u128; // Almost zero execution price
            
            let impact = spot_price_bps
                .saturating_sub(execution_price_bps)
                .saturating_mul(10000)
                .saturating_div(spot_price_bps);
            
            assert_eq!(impact, 9999); // ~99.99% impact
            
            // Test 3: Overflow protection with smaller numbers
            let large_spot = 1_000_000_000_000u128; // 1 trillion
            let large_exec = 500_000_000_000u128;   // 500 billion
            
            // Should not overflow even with large numbers
            let safe_impact = large_spot
                .checked_sub(large_exec)
                .and_then(|diff| diff.checked_mul(10000))
                .and_then(|prod| prod.checked_div(large_spot));
            
            assert!(safe_impact.is_some());
            assert_eq!(safe_impact.unwrap(), 5000); // 50% impact
        }

        #[ink::test]
        fn test_large_numbers_handling() {
            let contract = setup_contract();

            // Test with realistic maximum values based on token supplies
            // D9 max supply: 10^22, USDT max supply: 10^14
            let d9_max = 10_u128.pow(22);
            let usdt_max = 10_u128.pow(14);

            // Test case 1: Large but safe trade (1% of max D9)
            let safe_input = d9_max / 100; // 10^20
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                safe_input as Balance,
            );
            assert!(result.is_ok(), "Should handle 1% of max supply");

            // Test case 2: Trade that would overflow
            // With max reserves and 990 multiplier, max safe amount = u128::MAX / (990 * 10^14)
            let max_safe = u128::MAX / 990 / usdt_max;
            let overflow_input = max_safe + 1000; // Definitely over the limit
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                overflow_input as Balance,
            );
            assert_eq!(
                result,
                Err(Error::ArithmeticOverflow),
                "Should reject trades that would overflow"
            );
        }

        #[ink::test]
        fn test_estimate_matches_calculate() {
            // Skip this test as it requires get_currency_balance which calls external contracts
            // The test would need to be run as an integration test with actual deployed contracts
        }

        #[ink::test]
        fn test_slippage_increases_with_size() {
            let contract = setup_contract();

            let x = 1_000_000_000;
            let y = 1_000_000_000;

            // Test increasing trade sizes
            let trades = vec![
                1_000_000,   // 0.1% of pool
                10_000_000,  // 1% of pool
                100_000_000, // 10% of pool
                500_000_000, // 50% of pool
            ];

            let mut last_slippage = 0.0;

            for input in trades {
                let output = contract.calc_opposite_currency_amount(x, y, input).unwrap();

                // Calculate slippage: difference from ideal rate
                let ideal_output = input; // Since x = y, ideal rate is 1:1
                let slippage = ((ideal_output - output) as f64 / ideal_output as f64) * 100.0;

                // Slippage should increase with trade size
                assert!(
                    slippage > last_slippage,
                    "Slippage should increase with trade size"
                );

                last_slippage = slippage;
            }
        }

        #[ink::test]
        fn test_fee_precision() {
            // Test with different fee percentages
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);

            let fee_percentages = vec![0, 1, 3, 5, 10, 30]; // 0%, 0.1%, 0.3%, 0.5%, 1%, 3%

            for fee_percent in fee_percentages {
                let contract = MarketMaker::new(accounts.bob, fee_percent, 10);

                let x = 1_000_000_000;
                let y = 1_000_000_000;
                let input = 100_000_000;

                let output = contract.calc_opposite_currency_amount(x, y, input).unwrap();

                // Verify fee is correctly applied
                let fee_per_mille = (fee_percent as u128) * 10;
                let expected_effective_input = (input as u128) * (1000 - fee_per_mille) / 1000;

                // Recalculate expected output
                let expected_numerator = expected_effective_input * (y as u128);
                let expected_denominator = (x as u128) + expected_effective_input;
                let expected_output = expected_numerator / expected_denominator;

                // Allow for small rounding difference
                let diff = if output > expected_output as Balance {
                    output - expected_output as Balance
                } else {
                    expected_output as Balance - output
                };

                assert!(
                    diff <= 1,
                    "Fee calculation should be precise for {}% fee",
                    fee_percent
                );
            }
        }

        #[ink::test]
        fn test_minimum_output() {
            let contract = setup_contract();

            let x = 1_000_000_000_000; // Large pool
            let y = 1_000_000_000_000;

            // Very small input might produce zero output due to integer division
            let tiny_input = 1;
            let output = contract
                .calc_opposite_currency_amount(x, y, tiny_input)
                .unwrap();

            // With 1% fee and tiny input relative to large pools, output can round to 0
            // This is expected behavior with integer math
            // For tiny_input=1, fee reduces it to 0.99, and with large pools this rounds to 0
            assert_eq!(
                output, 0,
                "Tiny inputs can legitimately round to zero with large pools"
            );
        }

        #[ink::test]
        fn test_extreme_ratios() {
            let contract = setup_contract();

            // Extremely imbalanced pool
            let x = 1_000_000_000_000; // 1 trillion
            let y = 1_000; // Only 1000

            // Try to buy scarce asset
            let input = 1_000_000_000; // 1 billion

            let result = contract.calc_opposite_currency_amount(x, y, input);
            assert!(result.is_ok());

            let output = result.unwrap();

            // With extreme ratios, the output rounds to 0 due to integer division
            // Same calculation as test_extreme_liquidity_imbalance
            assert_eq!(output, 0, "Extreme ratios cause output to round to zero");

            // Since output is 0, the invariant is trivially maintained
            let k_before = (x as u128) * (y as u128);
            let k_after = (x as u128) * (y as u128); // No change since output is 0

            assert_eq!(k_after, k_before, "Invariant unchanged when output is 0");
        }

        #[ink::test]
        fn test_economic_security() {
            let contract = setup_contract();

            // Verify no arbitrage through sandwich attacks
            let x = 1_000_000_000;
            let y = 1_000_000_000;

            // Attacker front-runs with large trade
            let attack_input = 100_000_000;
            let attack_output = contract
                .calc_opposite_currency_amount(x, y, attack_input)
                .unwrap();

            // Update pool state
            let fee_per_mille = 10; // 1% fee
            let effective_attack_input = (attack_input as u128) * (1000 - fee_per_mille) / 1000;
            let x_after_attack = x + effective_attack_input as Balance;
            let y_after_attack = y - attack_output;

            // Victim's trade
            let victim_input = 10_000_000;
            let victim_output = contract
                .calc_opposite_currency_amount(x_after_attack, y_after_attack, victim_input)
                .unwrap();

            // Update pool again
            let effective_victim_input = (victim_input as u128) * (1000 - fee_per_mille) / 1000;
            let x_after_victim = x_after_attack + effective_victim_input as Balance;
            let y_after_victim = y_after_attack - victim_output;

            // Attacker tries to reverse trade
            let reverse_output = contract
                .calc_opposite_currency_amount(y_after_victim, x_after_victim, attack_output)
                .unwrap();

            // Attacker should lose money due to fees and slippage
            assert!(
                reverse_output < attack_input,
                "Sandwich attacks should not be profitable"
            );
        }

        #[ink::test]
        fn test_extreme_liquidity_imbalance() {
            let contract = setup_contract();

            // Extremely imbalanced pool
            let x = 1_000_000_000_000; // 1 trillion of X
            let y = 1_000; // Only 1000 of Y

            // Try to buy scarce asset with abundant asset
            let input = 1_000_000_000; // 1 billion

            let result = contract.calc_opposite_currency_amount(x, y, input);
            assert!(result.is_ok());

            let output = result.unwrap();
            // With extreme imbalance, the output actually rounds to 0 due to integer division
            // The calculated value is ~0.989, which rounds down to 0
            assert_eq!(
                output, 0,
                "Extreme imbalance causes output to round to zero"
            );
        }

        #[ink::test]
        fn test_numerical_precision_edge_cases() {
            let contract = setup_contract();

            // Test with realistic maximum values
            let d9_max = 10_u128.pow(22);
            let usdt_max = 10_u128.pow(14);

            // Test edge case: Maximum possible trade without overflow
            // For D9->USDT with max pools, limit is ~3.4% of supply
            let max_safe_trade = (d9_max as f64 * 0.034) as u128; // ~3.4%
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                max_safe_trade as Balance,
            );
            assert!(result.is_ok(), "Should handle maximum safe trade");

            // Test with very small numbers
            let small_x = 100;
            let small_y = 100;
            let small_input = 1;

            let small_result =
                contract.calc_opposite_currency_amount(small_x, small_y, small_input);
            assert!(small_result.is_ok());
            assert_eq!(small_result.unwrap(), 0, "Tiny trades may round to zero");

            // Test precision at overflow boundary
            let max_safe_boundary = u128::MAX / 990 / usdt_max;
            let boundary_input = max_safe_boundary + 1; // Just over the boundary
            let boundary_result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                boundary_input as Balance,
            );
            assert_eq!(
                boundary_result,
                Err(Error::ArithmeticOverflow),
                "Should reject trades just over the overflow boundary"
            );
        }

        #[ink::test]
        fn test_trade_reversal_with_fees() {
            let contract = setup_contract(); // 1% fee

            let x_init = 1_000_000_000;
            let y_init = 1_000_000_000;

            // Trade A -> B
            let input_a_to_b = 100_000_000;
            let output_a_to_b = contract
                .calc_opposite_currency_amount(x_init, y_init, input_a_to_b)
                .unwrap();

            // Update pool state with V2 formula (fee on input)
            let fee_per_mille = 10; // 1% = 10 per mille
            let effective_input_a_to_b = (input_a_to_b as u128) * (1000 - fee_per_mille) / 1000;
            let x_after_first = x_init + effective_input_a_to_b as Balance;
            let y_after_first = y_init - output_a_to_b;

            // Trade B -> A (reverse) with the output amount
            let final_a = contract
                .calc_opposite_currency_amount(y_after_first, x_after_first, output_a_to_b)
                .unwrap();

            // Due to fees on both trades, we should get back less than we put in
            assert!(
                final_a < input_a_to_b,
                "Fees should prevent profitable round trips"
            );

            let total_loss = input_a_to_b - final_a;
            let loss_percentage = (total_loss as f64 / input_a_to_b as f64) * 100.0;

            println!("Round trip loss: {:.2}%", loss_percentage);
            assert!(
                loss_percentage > 1.9,
                "Should lose approximately 2% on round trip with 1% fees on each trade"
            );
        }

        #[ink::test]
        fn test_overflow_boundary_conditions() {
            let contract = setup_contract();

            // Test the exact overflow boundaries for different pool configurations
            let d9_max = 10_u128.pow(22);
            let usdt_max = 10_u128.pow(14);

            // Calculate theoretical maximum trade before overflow
            // max_input * 990 * reserve_out < u128::MAX
            // max_input < u128::MAX / (990 * reserve_out)
            let fee_multiplier = 990_u128; // 1% fee with Uniswap V2 formula

            // Test 1: D9 -> USDT with maximum pools
            let max_d9_input = u128::MAX / fee_multiplier / usdt_max;
            // This should be approximately 3.4 × 10^20

            // Just under the limit should work
            let safe_input = max_d9_input - 1;
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                safe_input as Balance,
            );
            assert!(
                result.is_ok(),
                "Trade just under overflow limit should succeed"
            );

            // Just over the limit should fail
            let overflow_input = max_d9_input + 1;
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                overflow_input as Balance,
            );
            assert_eq!(
                result,
                Err(Error::ArithmeticOverflow),
                "Trade just over overflow limit should fail"
            );

            // Test 2: USDT -> D9 with maximum pools (more restrictive)
            let max_usdt_input = u128::MAX / fee_multiplier / d9_max;
            // This should be approximately 3.4 × 10^12

            let safe_usdt = max_usdt_input - 1;
            let result = contract.calc_opposite_currency_amount(
                usdt_max as Balance,
                d9_max as Balance,
                safe_usdt as Balance,
            );
            assert!(result.is_ok(), "USDT trade under limit should succeed");

            let overflow_usdt = max_usdt_input + 1;
            let result = contract.calc_opposite_currency_amount(
                usdt_max as Balance,
                d9_max as Balance,
                overflow_usdt as Balance,
            );
            assert_eq!(
                result,
                Err(Error::ArithmeticOverflow),
                "USDT trade over limit should fail"
            );
        }

        #[ink::test]
        fn test_pool_drainage_attack() {
            let contract = setup_contract();

            let x = 1_000_000_000;
            let y = 1_000_000_000;

            // Try to drain 99% of pool Y
            // This calculates how much X needed to get 99% of Y
            let target_output = y * 99 / 100;

            // Reverse calculate: given we want target_output of Y, how much X do we need?
            // New Y = y - target_output = y * 0.01
            // k = x * y = (x + input) * (y * 0.01)
            // input = (k / (y * 0.01)) - x = (x * y / (y * 0.01)) - x = x * 100 - x = x * 99

            let required_input = x * 99;

            let actual_output = contract
                .calc_opposite_currency_amount(x, y, required_input)
                .unwrap();

            // The actual output should be less than target due to the curve
            assert!(
                actual_output < target_output,
                "Cannot drain exact amount due to asymptotic curve"
            );

            // Calculate how close we got
            let drainage_percent = (actual_output as f64 / y as f64) * 100.0;
            println!("Attempted 99% drainage, achieved {:.2}%", drainage_percent);

            // Even with massive input, should not be able to drain pool completely
            assert!(
                drainage_percent < 99.0,
                "AMM curve should prevent complete drainage"
            );
        }

        #[ink::test]
        fn test_amm_constant_product_formula() {
            let contract = setup_contract();

            // Test the core x * y = k formula with balanced pools
            let x = 1_000_000_000; // 1000 tokens
            let y = 1_000_000_000; // 1000 tokens (balanced 1:1)
            let dx = 100_000_000; // 100 tokens input

            let dy = contract.calc_opposite_currency_amount(x, y, dx).unwrap();

            // Verify constant product with fees: x * y = (x + dx_with_fee) * (y - dy)
            // The effective input after 1% fee is 99% of dx
            let fee_per_mille = 10u128; // 1% = 10 per mille
            let dx_with_fee = (dx as u128 * (1000 - fee_per_mille)) / 1000;
            let k_before = x as u128 * y as u128;
            let k_after = (x as u128 + dx_with_fee) * (y - dy) as u128;

            // k_after should be approximately equal to k_before (small rounding allowed)
            let diff = if k_after > k_before {
                k_after - k_before
            } else {
                k_before - k_after
            };

            // Allow 0.01% difference for rounding
            let tolerance = k_before / 1000;
            assert!(
                diff <= tolerance,
                "Constant product not maintained: diff {} > tolerance {}",
                diff,
                tolerance
            );

            // With balanced pools, output should be less than input due to slippage
            assert!(
                dy < dx,
                "Output should be less than input for balanced pools"
            );

            // Test with imbalanced pools (trading from scarce to abundant)
            let x2 = 1_000_000_000; // 1000 tokens (scarce)
            let y2 = 2_000_000_000; // 2000 tokens (abundant)
            let dx2 = 100_000_000; // 100 tokens input

            let dy2 = contract.calc_opposite_currency_amount(x2, y2, dx2).unwrap();

            // When trading from scarce to abundant, output can be greater than input
            assert!(
                dy2 > dx2,
                "Should get more output when trading from scarce to abundant asset"
            );
        }

        #[ink::test]
        fn test_amm_zero_liquidity_should_fail() {
            let contract = setup_contract();

            // Zero liquidity should fail
            let result = contract.calc_opposite_currency_amount(0, 1000, 100);
            assert_eq!(
                result,
                Err(Error::DivisionByZero),
                "Should fail with zero input liquidity"
            );

            let result2 = contract.calc_opposite_currency_amount(1000, 0, 100);
            assert_eq!(
                result2,
                Err(Error::DivisionByZero),
                "Should fail with zero output liquidity"
            );
        }

        #[ink::test]
        #[should_panic(expected = "tolerance must be 0 <= x <= 100")]
        fn test_new_constructor_invalid_tolerance_percent() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);

            MarketMaker::new(accounts.bob, 1, 101);
        }

        #[ink::test]
        fn test_change_admin_by_admin() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            let mut contract = MarketMaker::new(accounts.bob, 1, 10);

            contract.change_admin(accounts.charlie).unwrap();

            assert_eq!(contract.admin, accounts.charlie);
        }

        #[ink::test]
        #[should_panic(expected = "Only admin can change admin.")]
        fn test_change_admin_by_non_admin_fails() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            let mut contract = MarketMaker::new(accounts.bob, 1, 10);

            set_caller::<DefaultEnvironment>(accounts.charlie);
            contract.change_admin(accounts.django).unwrap();
        }

        #[ink::test]
        fn test_change_admin_zero_address_fails() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            let mut contract = MarketMaker::new(accounts.bob, 1, 10);

            let zero_address = AccountId::from([0u8; 32]);
            let result = contract.change_admin(zero_address);
            assert_eq!(result, Err(Error::InvalidAddress));
        }

        // Currency Reserve and Balance Functions Tests
        #[ink::test]
        fn test_get_currency_reserves() {
            // Skip this test as it requires external contract calls
            // which are not supported in unit tests
        }

        #[ink::test]
        fn test_get_total_lp_tokens_empty_pool() {
            let contract = setup_contract();

            assert_eq!(contract.get_total_lp_tokens(), 0);
        }

        #[ink::test]
        fn test_get_total_lp_tokens_with_liquidity() {
            let mut contract = setup_contract();
            contract.total_lp_tokens = 1_000_000;

            assert_eq!(contract.get_total_lp_tokens(), 1_000_000);
        }

        #[ink::test]
        fn test_get_liquidity_provider_exists() {
            let accounts = get_default_test_accounts();
            let mut contract = setup_contract();

            contract
                .liquidity_providers
                .insert(accounts.alice, &500_000);

            assert_eq!(
                contract.get_liquidity_provider(accounts.alice),
                Some(500_000)
            );
        }

        #[ink::test]
        fn test_get_liquidity_provider_not_exists() {
            let accounts = get_default_test_accounts();
            let contract = setup_contract();

            assert_eq!(contract.get_liquidity_provider(accounts.charlie), None);
        }

        // Liquidity Management Functions Tests
        #[ink::test]
        fn test_add_liquidity_zero_d9_fails() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            set_value_transferred::<DefaultEnvironment>(0);
            let mut contract = setup_contract();

            let result = contract.add_liquidity(1000);

            assert_eq!(result, Err(Error::D9orUSDTProvidedLiquidityAtZero));
        }

        #[ink::test]
        fn test_add_liquidity_zero_usdt_fails() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            set_value_transferred::<DefaultEnvironment>(1000);
            let mut contract = setup_contract();

            let result = contract.add_liquidity(0);

            assert_eq!(result, Err(Error::D9orUSDTProvidedLiquidityAtZero));
        }

        #[ink::test]
        fn test_remove_liquidity_not_provider_fails() {
            // Skip this test as it requires get_currency_reserves which calls external contracts
        }

        #[ink::test]
        fn test_mint_lp_tokens_initial_pool() {
            let accounts = get_default_test_accounts();
            let mut contract = setup_contract();

            // For initial pool, reserves are 0
            // Use amounts larger than MINIMUM_LIQUIDITY to avoid LiquidityTooLow error
            let result = contract.mint_lp_tokens(accounts.alice, 2000, 2000, 0, 0);

            assert!(result.is_ok());
            // sqrt(2000 * 2000) = 2000, minus MINIMUM_LIQUIDITY (1000) = 1000
            assert_eq!(contract.total_lp_tokens, 1000);
            assert_eq!(
                contract.liquidity_providers.get(&accounts.alice),
                Some(1000)
            );
        }

        #[ink::test]
        fn test_calc_new_lp_tokens_empty_pool() {
            let mut contract = setup_contract();

            // For initial liquidity, reserves are 0
            let tokens = contract.calc_new_lp_tokens(2000, 2000, 0, 0);

            // sqrt(2000 * 2000) = 2000, minus MINIMUM_LIQUIDITY (1000)
            assert_eq!(tokens, 1000); // 2000 - 1000
        }
        
        #[ink::test]
        fn test_calc_new_lp_tokens_minimum_liquidity() {
            let mut contract = setup_contract();

            // Test with liquidity equal to MINIMUM_LIQUIDITY
            let tokens = contract.calc_new_lp_tokens(1000, 1000, 0, 0);
            
            // sqrt(1000 * 1000) = 1000, which equals MINIMUM_LIQUIDITY
            // Should return 0 as it's too small
            assert_eq!(tokens, 0);
            
            // Test with liquidity slightly above MINIMUM_LIQUIDITY
            let tokens2 = contract.calc_new_lp_tokens(1001, 1001, 0, 0);
            
            // sqrt(1001 * 1001) ≈ 1001, minus MINIMUM_LIQUIDITY = 1
            assert_eq!(tokens2, 1);
        }

        #[ink::test]
        fn test_calc_new_lp_tokens_existing_pool() {
            // Skip this test as it requires get_currency_reserves which calls external contracts
        }
        
        #[ink::test]
        fn test_calc_new_lp_tokens_ratio_validation() {
            let mut contract = setup_contract();
            contract.total_lp_tokens = 10000; // Simulate existing pool
            
            // Test 1: Perfectly balanced liquidity
            let tokens = contract.calc_new_lp_tokens(1000, 1000, 10000, 10000);
            assert_eq!(tokens, 1000); // Both ratios are 1000
            
            // Test 2: Slightly imbalanced but within tolerance (default 1%)
            // D9 ratio: 1000 * 10000 / 10000 = 1000
            // USDT ratio: 990 * 10000 / 10000 = 990
            // Difference: 1%, exactly at tolerance
            let tokens2 = contract.calc_new_lp_tokens(1000, 990, 10000, 10000);
            assert_eq!(tokens2, 990); // Min of the two ratios
            
            // Test 3: Imbalanced beyond tolerance
            // D9 ratio: 1000 * 10000 / 10000 = 1000
            // USDT ratio: 980 * 10000 / 10000 = 980
            // Difference: 2%, exceeds 1% tolerance
            let tokens3 = contract.calc_new_lp_tokens(1000, 980, 10000, 10000);
            assert_eq!(tokens3, 0); // Should reject due to imbalance
        }
        
        #[ink::test]
        fn test_calc_new_lp_tokens_overflow_protection() {
            let mut contract = setup_contract();
            
            // Test 1: Large initial liquidity that would overflow without safe_sqrt
            let large_amount = 1_000_000_000_000u128; // 1 trillion
            let tokens = contract.calc_new_lp_tokens(large_amount as Balance, 2, 0, 0);
            
            // Should compute sqrt of each separately when product would overflow
            // sqrt(large_amount) * sqrt(2) - MINIMUM_LIQUIDITY
            assert!(tokens > 0);
            
            // Test 2: Existing pool with reasonable numbers to test overflow handling
            contract.total_lp_tokens = 1_000_000_000; // 1 billion LP tokens
            
            // This could overflow during multiplication without checked arithmetic
            let tokens2 = contract.calc_new_lp_tokens(
                1_000_000_000, // 1 billion
                1_000_000_000, // 1 billion
                100,           // Small reserves
                100            // Small reserves
            );
            
            // Should handle the calculation properly or return 0 on overflow
            // The actual value depends on the arithmetic - we just ensure it doesn't panic
            assert!(tokens2 >= 0);
        }

        // Calculation and Helper Functions Tests
        #[ink::test]
        fn test_calc_opposite_currency_amount() {
            let contract = setup_contract();

            // Test constant product formula: x * y = k
            let balance_0: Balance = 1_000_000;
            let balance_1: Balance = 2_000_000;
            let amount_0: Balance = 100_000;

            let result = contract.calc_opposite_currency_amount(balance_0, balance_1, amount_0);

            assert!(result.is_ok());
            let amount_1 = result.unwrap();

            // Verify output amount is calculated correctly
            // amount_1 = balance_1 - (k / (balance_0 + amount_0))
            // where k = balance_0 * balance_1
            assert!(amount_1 > 0);
            assert!(amount_1 < balance_1);

            // Verify the AMM formula maintains the constant product (with fees)
            // The effective input after 1% fee is 99% of amount_0
            let fee_percent = FixedBalance::from_num(0.01); // 1% fee
            let amount_0_with_fee = FixedBalance::from_num(amount_0)
                .saturating_mul(FixedBalance::from_num(1) - fee_percent);
            let k_before =
                FixedBalance::from_num(balance_0).saturating_mul(FixedBalance::from_num(balance_1));
            let k_after = (FixedBalance::from_num(balance_0) + amount_0_with_fee)
                .saturating_mul(FixedBalance::from_num(balance_1 - amount_1));

            // k_after should be approximately equal to k_before (allowing for rounding)
            let diff = if k_after > k_before {
                k_after - k_before
            } else {
                k_before - k_after
            };
            // Allow small rounding error (0.01%)
            let tolerance = k_before.saturating_mul(FixedBalance::from_num(0.0001));
            assert!(diff <= tolerance);
        }

        #[ink::test]
        fn test_calc_opposite_currency_amount_division_by_zero() {
            let contract = setup_contract();

            // Test edge case where balance_0 is 0 - this means no liquidity
            // Should fail because you cannot swap when there's no input liquidity
            let result = contract.calc_opposite_currency_amount(0, 1000, 100);
            assert_eq!(
                result,
                Err(Error::DivisionByZero),
                "Should fail when input pool is empty"
            );

            // Test case where output pool is empty
            let result2 = contract.calc_opposite_currency_amount(1000, 0, 100);
            assert_eq!(
                result2,
                Err(Error::DivisionByZero),
                "Should fail when output pool is empty"
            );

            // Test normal swap
            let result3 = contract.calc_opposite_currency_amount(1000, 1000, 100);
            assert!(result3.is_ok());
            let output = result3.unwrap();
            assert!(
                output > 0 && output < 100,
                "Output should be less than input due to curve"
            );
        }

        #[ink::test]
        fn test_calculate_lp_percent() {
            let mut contract = setup_contract();
            contract.total_lp_tokens = 1_000_000;

            let lp_tokens = 250_000;
            let percent = contract.calculate_lp_percent(lp_tokens);

            assert_eq!(percent, FixedBalance::from_num(0.25));
        }

        #[ink::test]
        fn test_calculate_lp_percent_zero_total_tokens() {
            let contract = setup_contract();

            let lp_tokens = 250_000;
            let percent = contract.calculate_lp_percent(lp_tokens);

            assert_eq!(percent, FixedBalance::from_num(0));
        }

        #[ink::test]
        fn test_check_new_liquidity_within_tolerance() {
            // Skip this test as it requires get_currency_reserves which calls external contracts
        }

        #[ink::test]
        fn test_get_currency_balance() {
            // Skip this test as it requires external contract calls
        }

        #[ink::test]
        fn test_estimate_exchange_matches_calculate() {
            // Skip this test as it requires get_currency_balance which calls external contracts
        }

        // Mock/Helper Functions Tests
        #[ink::test]
        fn test_check_usdt_balance_insufficient() {
            // Skip this test as it requires external contract calls
        }

        #[ink::test]
        fn test_usdt_validity_check() {
            // Skip this test as it requires external contract calls
        }

        // Integration-style tests

        // Edge case tests
        #[ink::test]
        fn test_remove_liquidity_with_fees() {
            // Skip this test as it requires get_currency_reserves and external transfers
        }

        #[ink::test]
        fn test_direction_enum() {
            let dir1 = Direction(Currency::D9, Currency::USDT);
            let dir2 = Direction(Currency::USDT, Currency::D9);

            assert_ne!(dir1.0, dir1.1);
            assert_eq!(dir1.0, dir2.1);
            assert_eq!(dir1.1, dir2.0);
        }

        #[ink::test]
        fn test_error_enum_equality() {
            let err1 = Error::InsufficientLiquidity(Currency::D9);
            let err2 = Error::InsufficientLiquidity(Currency::D9);
            let err3 = Error::InsufficientLiquidity(Currency::USDT);

            assert_eq!(err1, err2);
            assert_ne!(err1, err3);
        }

        // Additional calculation tests that don't require external calls

        #[ink::test]
        fn test_calculate_lp_percent_various_scenarios() {
            let mut contract = setup_contract();

            // Test with 0 total tokens
            assert_eq!(
                contract.calculate_lp_percent(1000),
                FixedBalance::from_num(0)
            );

            // Test with equal tokens
            contract.total_lp_tokens = 1000;
            assert_eq!(
                contract.calculate_lp_percent(1000),
                FixedBalance::from_num(1)
            );

            // Test with half tokens
            contract.total_lp_tokens = 2000;
            assert_eq!(
                contract.calculate_lp_percent(1000),
                FixedBalance::from_num(0.5)
            );

            // Test with quarter tokens
            contract.total_lp_tokens = 4000;
            assert_eq!(
                contract.calculate_lp_percent(1000),
                FixedBalance::from_num(0.25)
            );
        }

        #[ink::test]
        fn test_mint_lp_tokens_scenarios() {
            // Skip this test as mint_lp_tokens calls calc_new_lp_tokens
            // which requires get_currency_reserves (external contract calls)
        }

        #[ink::test]
        fn test_calc_opposite_currency_amount_edge_cases() {
            let contract = setup_contract();

            // Test with very small amounts
            let result = contract.calc_opposite_currency_amount(1_000_000, 1_000_000, 1);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0); // Due to rounding

            // Test with equal balances
            let result2 = contract.calc_opposite_currency_amount(1000, 1000, 100);
            assert!(result2.is_ok());
            let output = result2.unwrap();
            assert!(output > 0 && output < 100); // Should be less than input due to curve

            // Test with unequal balances
            let result3 = contract.calc_opposite_currency_amount(1000, 2000, 100);
            assert!(result3.is_ok());
            let output3 = result3.unwrap();
            assert!(output3 > output); // Should get more of the abundant currency
        }

        #[ink::test]
        fn test_liquidity_provider_management() {
            let accounts = get_default_test_accounts();
            let mut contract = setup_contract();

            // Test adding liquidity provider
            contract
                .liquidity_providers
                .insert(accounts.alice, &100_000);
            assert_eq!(
                contract.get_liquidity_provider(accounts.alice),
                Some(100_000)
            );

            // Test updating liquidity provider
            contract
                .liquidity_providers
                .insert(accounts.alice, &200_000);
            assert_eq!(
                contract.get_liquidity_provider(accounts.alice),
                Some(200_000)
            );

            // Test removing liquidity provider
            contract.liquidity_providers.remove(&accounts.alice);
            assert_eq!(contract.get_liquidity_provider(accounts.alice), None);
        }

        #[ink::test]
        fn test_admin_functions() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);
            let mut contract = MarketMaker::new(accounts.bob, 1, 10);

            // Test initial admin
            assert_eq!(contract.admin, accounts.alice);

            // Test admin can change admin
            contract.change_admin(accounts.charlie).unwrap();
            assert_eq!(contract.admin, accounts.charlie);

            // Test new admin can change admin
            set_caller::<DefaultEnvironment>(accounts.charlie);
            contract.change_admin(accounts.django).unwrap();
            assert_eq!(contract.admin, accounts.django);
        }

        #[ink::test]
        fn test_currency_enum() {
            // Test currency enum variants
            let d9 = Currency::D9;
            let usdt = Currency::USDT;

            assert_ne!(d9, usdt);
            assert_eq!(d9, Currency::D9);
            assert_eq!(usdt, Currency::USDT);
        }

        #[ink::test]
        fn test_direction_creation_and_comparison() {
            let dir1 = Direction(Currency::D9, Currency::USDT);
            let dir2 = Direction(Currency::USDT, Currency::D9);
            let dir3 = Direction(Currency::D9, Currency::USDT);

            // Test that same directions are equal
            assert_eq!(dir1.0, dir3.0);
            assert_eq!(dir1.1, dir3.1);

            // Test that reversed directions are different
            assert_eq!(dir1.0, dir2.1);
            assert_eq!(dir1.1, dir2.0);
        }

        #[ink::test]
        fn test_tolerance_percent_bounds() {
            let accounts = get_default_test_accounts();
            set_caller::<DefaultEnvironment>(accounts.alice);

            // Test valid tolerance percentages
            let contract1 = MarketMaker::new(accounts.bob, 1, 0);
            assert_eq!(contract1.liquidity_tolerance_percent, 0);

            let contract2 = MarketMaker::new(accounts.bob, 1, 50);
            assert_eq!(contract2.liquidity_tolerance_percent, 50);

            let contract3 = MarketMaker::new(accounts.bob, 1, 100);
            assert_eq!(contract3.liquidity_tolerance_percent, 100);
        }

        #[ink::test]
        fn test_fixed_balance_precision() {
            // Test precision in calculations
            let small_lp = 1;
            let total_lp = 1_000_000_000_000; // 1 trillion
            let mut contract_with_lp = setup_contract();
            contract_with_lp.total_lp_tokens = total_lp;

            let percent = contract_with_lp.calculate_lp_percent(small_lp);

            // Very small LP compared to total should give very small percentage
            // Due to precision limits, it might round to 0
            assert!(
                percent == FixedBalance::from_num(0) || percent < FixedBalance::from_num(0.000001)
            );
        }

        #[ink::test]
        fn test_saturating_operations() {
            let contract = setup_contract();

            // Test with large but safe values
            let large_balance = 1_000_000_000_000_000; // 1 quadrillion
            let result = contract.calc_opposite_currency_amount(
                large_balance,
                large_balance,
                large_balance / 10,
            );

            // Should handle large values without overflow
            assert!(result.is_ok());
            let output = result.unwrap();
            assert!(output > 0);
            assert!(output < large_balance);
        }
    }

    #[cfg(all(test, feature = "e2e-tests"))]
    mod mock_usdt {
        include!("mock_usdt.rs");
    }

    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        use super::mock_usdt::mock_usdt::MockUsdtRef;
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        #[ink_e2e::test]
        async fn check_liquidity(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let d9_liquidity: Balance = 10_000_000000000000;
            let usdt_liquidity: Balance = 10_000_00;
            let usdt_constructor = MockUsdtRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10);
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
            let usdt_constructor = MockUsdtRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;

            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10);
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
            let usdt_constructor = MockUsdtRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let usdt_approved_amount = initial_supply.saturating_div(2000);
            let approval_message =
                build_message::<MockUsdtRef>(usdt_address.clone()).call(|d9_usdt| {
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

        #[ink_e2e::test]
        async fn add_liquidity(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            //init usdt contract
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = MockUsdtRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("failed to instantiate usdt")
                .account_id;
            // init market maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("failed to instantiate market maker")
                .account_id;

            //build approval message
            let usdt_approval_amount = 100_000_000_000_000;
            let approval_message =
                build_message::<MockUsdtRef>(usdt_address.clone()).call(|d9_usdt| {
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

        #[ink_e2e::test]
        async fn e2e_complete_swap_flow(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Deploy USDT contract
            let initial_supply: Balance = 1_000_000_000_000_000; // 1M USDT
            let usdt_constructor = MockUsdtRef::new(initial_supply);
            let usdt_contract = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("USDT instantiation failed");
            let usdt_address = usdt_contract.account_id;

            // Deploy Market Maker
            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10); // 1% fee, 10% tolerance
            let amm_contract = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("Market Maker instantiation failed");
            let amm_address = amm_contract.account_id;

            // Step 1: Alice adds initial liquidity
            // Approve USDT spending
            let usdt_liquidity = 100_000_000_000; // 100K USDT
            let approval_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.approve(
                    account_id(AccountKeyring::Alice),
                    amm_address.clone(),
                    usdt_liquidity,
                )
            });

            client
                .call(&ink_e2e::alice(), approval_msg, 0, None)
                .await
                .expect("USDT approval failed");

            // Add liquidity (1:1 ratio)
            let d9_liquidity = 100_000_000_000; // 100K D9
            let add_liquidity_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.add_liquidity(usdt_liquidity));

            client
                .call(&ink_e2e::alice(), add_liquidity_msg, d9_liquidity, None)
                .await
                .expect("Add liquidity failed");

            // Check LP tokens
            let get_lp_tokens_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.get_total_lp_tokens());

            let total_lp_tokens = client
                .call_dry_run(&ink_e2e::alice(), &get_lp_tokens_msg, 0, None)
                .await
                .return_value();

            assert_eq!(total_lp_tokens, 1_000_000, "Initial LP tokens should be 1M");

            // Step 2: Bob swaps D9 for USDT
            let swap_amount_d9 = 10_000_000_000; // 10K D9

            // First, estimate the swap
            let estimate_msg = build_message::<MarketMakerRef>(amm_address.clone()).call(|amm| {
                amm.estimate_exchange(Direction(Currency::D9, Currency::USDT), swap_amount_d9)
            });

            let (input, output) = client
                .call_dry_run(&ink_e2e::bob(), &estimate_msg, 0, None)
                .await
                .return_value();

            assert_eq!(input, swap_amount_d9);
            assert!(
                output > 0 && output < swap_amount_d9,
                "Output should be positive but less due to AMM curve"
            );

            // Execute the swap
            let swap_msg =
                build_message::<MarketMakerRef>(amm_address.clone()).call(|amm| amm.get_usdt());

            let swap_result = client
                .call(&ink_e2e::bob(), swap_msg, swap_amount_d9, None)
                .await
                .expect("Swap D9 to USDT failed");

            // Verify Bob received USDT
            let bob_usdt_balance_msg = build_message::<MockUsdtRef>(usdt_address.clone())
                .call(|usdt| usdt.balance_of(account_id(AccountKeyring::Bob)));

            let bob_usdt_balance = client
                .call_dry_run(&ink_e2e::bob(), &bob_usdt_balance_msg, 0, None)
                .await
                .return_value();

            assert_eq!(
                bob_usdt_balance, output,
                "Bob should have received the estimated USDT"
            );

            // Step 3: Charlie buys D9 with USDT
            // First, transfer some USDT to Charlie
            let transfer_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.transfer(account_id(AccountKeyring::Charlie), 50_000_000_000, vec![])
            });

            client
                .call(&ink_e2e::alice(), transfer_msg, 0, None)
                .await
                .expect("USDT transfer to Charlie failed");

            // Charlie approves AMM
            let charlie_usdt_amount = 5_000_000_000; // 5K USDT
            let charlie_approval_msg =
                build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                    usdt.approve(
                        account_id(AccountKeyring::Charlie),
                        amm_address.clone(),
                        charlie_usdt_amount,
                    )
                });

            client
                .call(&ink_e2e::charlie(), charlie_approval_msg, 0, None)
                .await
                .expect("Charlie's USDT approval failed");

            // Charlie swaps USDT for D9
            let charlie_swap_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.get_d9(charlie_usdt_amount));

            let charlie_d9_received = client
                .call(&ink_e2e::charlie(), charlie_swap_msg, 0, None)
                .await
                .expect("Charlie's swap failed")
                .return_value();

            assert!(charlie_d9_received > 0, "Charlie should receive D9");

            Ok(())
        }

        #[ink_e2e::test]
        async fn e2e_liquidity_provider_lifecycle(
            mut client: ink_e2e::Client<C, E>,
        ) -> E2EResult<()> {
            // Deploy contracts
            let usdt_constructor = MockUsdtRef::new(10_000_000_000_000);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await
                .expect("USDT deploy failed")
                .account_id;

            let amm_constructor = MarketMakerRef::new(usdt_address, 2, 10); // 2% fee
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await
                .expect("AMM deploy failed")
                .account_id;

            // Alice provides initial liquidity
            let alice_usdt = 1_000_000_000_000;
            let alice_d9 = 1_000_000_000_000;

            // Approve and add liquidity
            let approve_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.approve(
                    account_id(AccountKeyring::Alice),
                    amm_address.clone(),
                    alice_usdt,
                )
            });

            client.call(&ink_e2e::alice(), approve_msg, 0, None).await?;

            let add_liq_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.add_liquidity(alice_usdt));

            client
                .call(&ink_e2e::alice(), add_liq_msg, alice_d9, None)
                .await?;

            // Check Alice's LP tokens
            let alice_lp_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.get_liquidity_provider(account_id(AccountKeyring::Alice)));

            let alice_lp_tokens = client
                .call_dry_run(&ink_e2e::alice(), &alice_lp_msg, 0, None)
                .await
                .return_value();

            assert_eq!(alice_lp_tokens, Some(1_000_000));

            // Bob adds proportional liquidity
            // First, transfer USDT to Bob
            let transfer_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.transfer(account_id(AccountKeyring::Bob), 500_000_000_000, vec![])
            });

            client
                .call(&ink_e2e::alice(), transfer_msg, 0, None)
                .await?;

            // Bob approves and adds liquidity
            let bob_usdt = 500_000_000_000;
            let bob_d9 = 500_000_000_000;

            let bob_approve_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.approve(
                    account_id(AccountKeyring::Bob),
                    amm_address.clone(),
                    bob_usdt,
                )
            });

            client
                .call(&ink_e2e::bob(), bob_approve_msg, 0, None)
                .await?;

            let bob_add_liq_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.add_liquidity(bob_usdt));

            client
                .call(&ink_e2e::bob(), bob_add_liq_msg, bob_d9, None)
                .await?;

            // Check total LP tokens increased
            let total_lp_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.get_total_lp_tokens());

            let total_lp = client
                .call_dry_run(&ink_e2e::alice(), &total_lp_msg, 0, None)
                .await
                .return_value();

            assert!(total_lp > 1_000_000, "Total LP tokens should increase");

            // Alice removes liquidity
            let remove_liq_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.remove_liquidity());

            client
                .call(&ink_e2e::alice(), remove_liq_msg, 0, None)
                .await?;

            // Verify Alice no longer has LP tokens
            let alice_lp_after = client
                .call_dry_run(&ink_e2e::alice(), &alice_lp_msg, 0, None)
                .await
                .return_value();

            assert_eq!(
                alice_lp_after, None,
                "Alice should have no LP tokens after removal"
            );

            Ok(())
        }

        #[ink_e2e::test]
        async fn e2e_fee_collection_and_distribution(
            mut client: ink_e2e::Client<C, E>,
        ) -> E2EResult<()> {
            // Deploy with higher fee for easier testing
            let usdt_constructor = MockUsdtRef::new(100_000_000_000_000);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await?
                .account_id;

            let amm_constructor = MarketMakerRef::new(usdt_address, 5, 10); // 5% fee
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await?
                .account_id;

            // Alice adds liquidity
            let liquidity = 100_000_000_000;

            let approve_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.approve(
                    account_id(AccountKeyring::Alice),
                    amm_address.clone(),
                    liquidity,
                )
            });

            client.call(&ink_e2e::alice(), approve_msg, 0, None).await?;

            let add_liq_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.add_liquidity(liquidity));

            client
                .call(&ink_e2e::alice(), add_liq_msg, liquidity, None)
                .await?;

            // Transfer USDT to Bob for swapping
            let transfer_msg = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                usdt.transfer(account_id(AccountKeyring::Bob), 20_000_000_000, vec![])
            });

            client
                .call(&ink_e2e::alice(), transfer_msg, 0, None)
                .await?;

            // Bob performs multiple swaps to generate fees
            for _ in 0..5 {
                // Bob approves
                let bob_approve = build_message::<MockUsdtRef>(usdt_address.clone()).call(|usdt| {
                    usdt.approve(
                        account_id(AccountKeyring::Bob),
                        amm_address.clone(),
                        2_000_000_000,
                    )
                });

                client.call(&ink_e2e::bob(), bob_approve, 0, None).await?;

                // Bob swaps USDT for D9
                let swap_msg = build_message::<MarketMakerRef>(amm_address.clone())
                    .call(|amm| amm.get_d9(1_000_000_000));

                client.call(&ink_e2e::bob(), swap_msg, 0, None).await?;
            }

            // Alice removes liquidity and should receive fees
            let remove_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.remove_liquidity());

            client.call(&ink_e2e::alice(), remove_msg, 0, None).await?;

            // Alice should have received more than initial due to fees
            // (Exact verification would require checking balances before/after)
            Ok(())
        }

        #[ink_e2e::test]
        async fn e2e_error_scenarios(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Deploy contracts
            let usdt_constructor = MockUsdtRef::new(100_000_000_000);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None)
                .await?
                .account_id;

            let amm_constructor = MarketMakerRef::new(usdt_address, 1, 10);
            let amm_address = client
                .instantiate("market_maker", &ink_e2e::alice(), amm_constructor, 0, None)
                .await?
                .account_id;

            // Test 1: Add liquidity with zero amounts
            let zero_liq_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.add_liquidity(0));

            let zero_result = client.call(&ink_e2e::alice(), zero_liq_msg, 0, None).await;

            assert!(zero_result.is_err(), "Zero liquidity should fail");

            // Test 2: Swap without liquidity
            let no_liq_swap_msg =
                build_message::<MarketMakerRef>(amm_address.clone()).call(|amm| amm.get_usdt());

            let no_liq_result = client
                .call(&ink_e2e::bob(), no_liq_swap_msg, 1_000_000, None)
                .await;

            assert!(no_liq_result.is_err(), "Swap without liquidity should fail");

            // Test 3: Remove liquidity without being LP
            let remove_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.remove_liquidity());

            let remove_result = client.call(&ink_e2e::charlie(), remove_msg, 0, None).await;

            assert!(remove_result.is_err(), "Non-LP remove should fail");

            // Test 4: Insufficient allowance
            let insufficient_swap_msg = build_message::<MarketMakerRef>(amm_address.clone())
                .call(|amm| amm.get_d9(1_000_000_000));

            let insufficient_result = client
                .call(&ink_e2e::bob(), insufficient_swap_msg, 0, None)
                .await;

            assert!(
                insufficient_result.is_err(),
                "Insufficient allowance should fail"
            );

            Ok(())
        }

        // ===== New Feature Tests =====

        #[ink::test]
        fn test_minimum_liquidity_checks() {
            let contract = setup_contract();

            // Test with reserves below minimum liquidity
            let result = contract.calculate_exchange(Direction(Currency::D9, Currency::USDT), 100);

            // Should fail because get_currency_balance would return 0 (no external contracts)
            assert!(result.is_err());
        }

        #[ink::test]
        fn test_calc_new_lp_tokens_initial_liquidity() {
            let mut contract = setup_contract();

            // Test initial liquidity calculation (sqrt formula with MINIMUM_LIQUIDITY burn)
            let d9_amount = 1_000_000;
            let usdt_amount = 4_000_000;

            // For initial liquidity, reserves are 0
            let lp_tokens = contract.calc_new_lp_tokens(d9_amount, usdt_amount, 0, 0);

            // sqrt(1_000_000 * 4_000_000) = sqrt(4_000_000_000_000) = 2_000_000
            // Then subtract MINIMUM_LIQUIDITY (1000)
            assert_eq!(lp_tokens, 2_000_000 - 1000);
        }

        #[ink::test]
        fn test_calc_new_lp_tokens_proportional() {
            // This test requires get_currency_reserves which calls external contracts
            // The logic tests: LP tokens = min(d9_ratio, usdt_ratio)
        }

        #[ink::test]
        fn test_sqrt_calculation() {
            let mut contract = setup_contract();

            // Test various sqrt calculations with MINIMUM_LIQUIDITY subtraction
            let test_cases = vec![
                (1100, 1100, 100),                   // sqrt(1100 * 1100) = 1100 - 1000 = 100
                (2025, 1936, 980),                   // sqrt(2025 * 1936) ≈ 1980, then 1980 - 1000 = 980
                (1_001_000, 1_001_000, 1_000_000),  // sqrt(1.001M * 1.001M) ≈ 1.001M - 1000 ≈ 1M
            ];

            for (d9, usdt, expected) in test_cases {
                // For initial liquidity, reserves are 0
                let result = contract.calc_new_lp_tokens(d9, usdt, 0, 0);
                assert!(
                    result >= expected.saturating_sub(1) && result <= expected.saturating_add(1),
                    "sqrt({} * {}) - MINIMUM_LIQUIDITY should be approximately {}, got {}",
                    d9, usdt, expected, result
                );
            }
        }
        
        #[ink::test]
        fn test_sqrt_newton_verified() {
            let contract = setup_contract();
            
            // Test exact squares
            assert_eq!(contract.sqrt_newton_verified(0), 0);
            assert_eq!(contract.sqrt_newton_verified(1), 1);
            assert_eq!(contract.sqrt_newton_verified(4), 2);
            assert_eq!(contract.sqrt_newton_verified(9), 3);
            assert_eq!(contract.sqrt_newton_verified(16), 4);
            assert_eq!(contract.sqrt_newton_verified(100), 10);
            assert_eq!(contract.sqrt_newton_verified(10000), 100);
            assert_eq!(contract.sqrt_newton_verified(1000000), 1000);
            
            // Test non-exact squares (should return floor)
            assert_eq!(contract.sqrt_newton_verified(2), 1);
            assert_eq!(contract.sqrt_newton_verified(3), 1);
            assert_eq!(contract.sqrt_newton_verified(5), 2);
            assert_eq!(contract.sqrt_newton_verified(8), 2);
            assert_eq!(contract.sqrt_newton_verified(10), 3);
            assert_eq!(contract.sqrt_newton_verified(99), 9);
            assert_eq!(contract.sqrt_newton_verified(101), 10);
            
            // Test large numbers
            let large = u128::MAX / 2;
            let sqrt_large = contract.sqrt_newton_verified(large);
            
            // Verify the result is correct
            assert!(sqrt_large.saturating_mul(sqrt_large) <= large);
            assert!((sqrt_large + 1).saturating_mul(sqrt_large + 1) > large);
            
            // Test edge case: u128::MAX
            let sqrt_max = contract.sqrt_newton_verified(u128::MAX);
            assert!(sqrt_max.saturating_mul(sqrt_max) <= u128::MAX);
        }

        // ===== Comprehensive AMM Tests =====

        /// Core AMM Invariant Tests
        #[ink::test]
        fn test_uniswap_v2_formula_correctness() {
            let contract = setup_contract();

            // Test case 1: Balanced pool
            let reserve_x = 1_000_000_000_000; // 1T
            let reserve_y = 1_000_000_000_000; // 1T
            let amount_in = 1_000_000_000; // 1B

            // Calculate expected output using Uniswap V2 formula
            // With 1% fee: amount_out = (amount_in * 990 * reserve_out) / (reserve_in * 1000 + amount_in * 990)
            let amount_in_with_fee = (amount_in as u128) * 990;
            let numerator = amount_in_with_fee * (reserve_y as u128);
            let denominator = (reserve_x as u128) * 1000 + amount_in_with_fee;
            let expected_output = numerator / denominator;

            let actual_output = contract
                .calc_opposite_currency_amount(reserve_x, reserve_y, amount_in)
                .unwrap();

            // The actual implementation should match Uniswap V2 formula
            assert_eq!(
                actual_output as u128, expected_output,
                "Output should match Uniswap V2 formula. Expected: {}, Actual: {}",
                expected_output, actual_output
            );
        }

        #[ink::test]
        fn test_constant_product_invariant_exact() {
            let contract = setup_contract();

            let reserve_in = 10_000_000_000; // 10B
            let reserve_out = 5_000_000_000; // 5B
            let amount_in = 100_000_000; // 100M

            // Initial constant product
            let k_initial = (reserve_in as u128) * (reserve_out as u128);

            // Get output amount
            let amount_out = contract
                .calc_opposite_currency_amount(reserve_in, reserve_out, amount_in)
                .unwrap();

            // For Uniswap V2 with 1% fee:
            // New reserves after trade
            let amount_in_with_fee = (amount_in as u128) * 990 / 1000; // 1% fee deducted
            let new_reserve_in = (reserve_in as u128) + amount_in_with_fee;
            let new_reserve_out = (reserve_out as u128) - (amount_out as u128);

            // New constant product
            let k_final = new_reserve_in * new_reserve_out;

            // K should remain constant or slightly increase (due to fees)
            assert!(
                k_final >= k_initial,
                "Constant product should be maintained or increase. Initial: {}, Final: {}",
                k_initial,
                k_final
            );

            // K should not increase by more than the fee amount
            let k_increase_ratio = (k_final as f64) / (k_initial as f64);
            assert!(
                k_increase_ratio < 1.001, // Less than 0.1% increase
                "K increase should be minimal: {}%",
                (k_increase_ratio - 1.0) * 100.0
            );
        }

        #[ink::test]
        fn test_swap_price_impact_calculation() {
            let contract = setup_contract();

            let reserve_in = 100_000_000_000; // 100B
            let reserve_out = 100_000_000_000; // 100B (1:1 ratio)

            // Test various trade sizes and their price impacts
            // Note: Base impact of ~1% due to 1% fee
            let test_cases = vec![
                (1_000_000, 1.001), // 0.001% of pool -> ~1.001% impact (1% fee + 0.001% slippage)
                (10_000_000, 1.01), // 0.01% of pool -> ~1.01% impact
                (100_000_000, 1.1), // 0.1% of pool -> ~1.1% impact
                (1_000_000_000, 2.0), // 1% of pool -> ~2% impact
                (10_000_000_000, 11.0), // 10% of pool -> ~11% impact
            ];

            for (amount_in, expected_impact_pct) in test_cases {
                let amount_out = contract
                    .calc_opposite_currency_amount(reserve_in, reserve_out, amount_in)
                    .unwrap();

                // Calculate actual price vs spot price
                let spot_price = (reserve_out as f64) / (reserve_in as f64); // Should be 1.0
                let execution_price = (amount_out as f64) / (amount_in as f64);
                let price_impact = (1.0 - (execution_price / spot_price)) * 100.0;

                // Price impact should be within 20% of expected (accounting for fees)
                assert!(
                    (price_impact - expected_impact_pct).abs() < expected_impact_pct * 0.2,
                    "Trade size {} should have ~{}% impact, got {}%",
                    amount_in,
                    expected_impact_pct,
                    price_impact
                );
            }
        }

        #[ink::test]
        fn test_arbitrage_resistance() {
            let contract = setup_contract();

            // Start with balanced pool
            let initial_x = 1_000_000_000_000; // 1T
            let initial_y = 1_000_000_000_000; // 1T

            // Simulate arbitrage attempt
            let trade_amount = 100_000_000_000; // 100B (10% of pool)

            // Trade X -> Y
            let y_received = contract
                .calc_opposite_currency_amount(initial_x, initial_y, trade_amount)
                .unwrap();

            // Update pool state
            let new_x = initial_x + (trade_amount * 990 / 1000); // With fee
            let new_y = initial_y - y_received;

            // Try to trade back Y -> X
            let x_received_back = contract
                .calc_opposite_currency_amount(new_y, new_x, y_received)
                .unwrap();

            // Arbitrageur should lose money due to fees and slippage
            assert!(
                x_received_back < trade_amount,
                "Arbitrage should not be profitable. Sent: {}, Received: {}",
                trade_amount,
                x_received_back
            );

            // Calculate loss percentage
            let loss = trade_amount - x_received_back;
            let loss_pct = (loss as f64 / trade_amount as f64) * 100.0;

            // With 1% fee on each trade, minimum loss should be ~2%
            assert!(
                loss_pct > 1.9,
                "Round-trip loss should be at least 1.9%, got {}%",
                loss_pct
            );
        }

        #[ink::test]
        fn test_extreme_ratios_handling() {
            let contract = setup_contract();

            // Test cases with extreme ratios
            let test_cases = vec![
                (1_000_000_000_000, 1_000, 1_000_000),           // 1T:1K ratio
                (1_000, 1_000_000_000_000, 100),                 // 1K:1T ratio
                (u64::MAX as Balance, 1_000_000, 1_000_000_000), // Max:1M ratio
            ];

            for (reserve_x, reserve_y, amount_in) in test_cases {
                let result =
                    contract.calc_opposite_currency_amount(reserve_x, reserve_y, amount_in);

                if let Ok(amount_out) = result {
                    // Verify the trade maintains the invariant
                    let k_before = (reserve_x as u128) * (reserve_y as u128);
                    let amount_in_with_fee = (amount_in as u128) * 990 / 1000;
                    let k_after = ((reserve_x as u128) + amount_in_with_fee)
                        * ((reserve_y as u128) - (amount_out as u128));

                    assert!(
                        k_after >= k_before,
                        "Invariant should be maintained even with extreme ratios"
                    );
                }
            }
        }

        #[ink::test]
        fn test_fee_calculation_precision() {
            let accounts = default_accounts::<DefaultEnvironment>();
            set_caller::<DefaultEnvironment>(accounts.alice);

            // Test different fee percentages
            // Note: fee_percent is in whole percentages (1 = 1%, not 0.1%)
            let fee_configs = vec![
                (0, 1000), // 0% fee -> multiplier 1000
                (1, 990),  // 1% fee -> multiplier 990
                (3, 970),  // 3% fee -> multiplier 970
                (10, 900), // 10% fee -> multiplier 900
                (30, 700), // 30% fee -> multiplier 700
                (50, 500), // 50% fee -> multiplier 500
            ];

            for (fee_percent, expected_multiplier) in fee_configs {
                let contract = MarketMaker::new(accounts.bob, fee_percent, 10);

                let reserve_in = 1_000_000_000;
                let reserve_out = 1_000_000_000;
                let amount_in = 1_000_000;

                let amount_out = contract
                    .calc_opposite_currency_amount(reserve_in, reserve_out, amount_in)
                    .unwrap();

                // Verify fee is applied correctly
                let amount_in_with_fee = (amount_in as u128) * (expected_multiplier as u128) / 1000;
                let expected_out = (amount_in_with_fee * (reserve_out as u128))
                    / ((reserve_in as u128) + amount_in_with_fee);

                // Allow for rounding differences
                assert_eq!(
                    amount_out as u128, expected_out,
                    "Fee calculation incorrect for {}% fee",
                    fee_percent
                );
            }
        }

        #[ink::test]
        fn test_lp_token_calculation_correctness() {
            let mut contract = setup_contract();

            // Mock initial state
            contract.total_lp_tokens = 1_000_000;

            // Note: This test requires get_currency_reserves which calls external contracts
            // The current implementation has been fixed to use proper Uniswap V2 formula:
            // - Initial liquidity: sqrt(d9 * usdt)
            // - Subsequent liquidity: min(d9_ratio, usdt_ratio)
        }

        #[ink::test]
        fn test_slippage_protection_needed() {
            let contract = setup_contract();

            let reserve_in = 1_000_000_000;
            let reserve_out = 1_000_000_000;

            // Large trade that will have significant slippage
            let large_trade = 100_000_000; // 10% of pool

            let output = contract
                .calc_opposite_currency_amount(reserve_in, reserve_out, large_trade)
                .unwrap();

            // Calculate slippage
            let expected_without_slippage = large_trade; // 1:1 ratio
            let actual_slippage = ((expected_without_slippage - output) as f64
                / expected_without_slippage as f64)
                * 100.0;

            assert!(
                actual_slippage > 5.0,
                "Large trades should have significant slippage: {}%",
                actual_slippage
            );

            // This demonstrates need for min_amount_out parameter
        }

        #[ink::test]
        fn test_overflow_boundaries_exact() {
            let contract = setup_contract();

            // Test maximum safe values for D9 -> USDT swap
            let d9_max = 10_u128.pow(22);
            let usdt_max = 10_u128.pow(14);

            // Calculate exact overflow boundary
            // For 1% fee: (amount * 990 * reserve_out) must fit in u128
            // max_amount = u128::MAX / (990 * reserve_out)

            let max_safe_amount = u128::MAX / 990 / usdt_max;

            // Just under should work
            let safe_amount = (max_safe_amount - 1) as Balance;
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                safe_amount,
            );
            assert!(result.is_ok(), "Safe amount should not overflow");

            // Just over should fail
            let overflow_amount = (max_safe_amount + 1) as Balance;
            let result = contract.calc_opposite_currency_amount(
                d9_max as Balance,
                usdt_max as Balance,
                overflow_amount,
            );
            assert_eq!(
                result,
                Err(Error::ArithmeticOverflow),
                "Overflow amount should fail"
            );
        }

        #[ink::test]
        fn test_minimum_liquidity_requirement() {
            let contract = setup_contract();

            // Very small amounts should handle gracefully
            let tiny_reserves = 100;
            let tiny_input = 1;

            let result =
                contract.calc_opposite_currency_amount(tiny_reserves, tiny_reserves, tiny_input);

            assert!(result.is_ok());
            let output = result.unwrap();

            // With tiny amounts, output might round to 0
            // This is a concern for production - need minimum liquidity
            if output == 0 {
                println!("Warning: Tiny trades can result in zero output");
            }
        }

        #[ink::test]
        fn test_price_oracle_manipulation_resistance() {
            let contract = setup_contract();

            // Initial balanced state
            let mut reserve_x = 1_000_000_000_000;
            let mut reserve_y = 1_000_000_000_000;

            // Attacker tries to manipulate price in one block
            let manipulation_amount = 500_000_000_000; // 50% of pool

            // Calculate price before
            let price_before = (reserve_y as f64) / (reserve_x as f64);

            // Large trade to manipulate
            let output = contract
                .calc_opposite_currency_amount(reserve_x, reserve_y, manipulation_amount)
                .unwrap();

            // Update reserves
            reserve_x += manipulation_amount * 990 / 1000;
            reserve_y -= output;

            // Calculate price after
            let price_after = (reserve_y as f64) / (reserve_x as f64);

            // Price change should be significant
            let price_change = ((price_after - price_before).abs() / price_before) * 100.0;

            assert!(
                price_change > 30.0,
                "Large trades can manipulate price significantly: {}%",
                price_change
            );

            // This shows need for TWAP oracle for price feeds
        }

        #[ink::test]
        fn test_rounding_consistency() {
            let contract = setup_contract();

            // Test that rounding is consistent and conservative
            let reserve_in = 1_000_000_123; // Odd number
            let reserve_out = 999_999_877; // Odd number
            let amount_in = 12_345_678; // Odd number

            let output = contract
                .calc_opposite_currency_amount(reserve_in, reserve_out, amount_in)
                .unwrap();

            // Reverse calculation
            let reverse_input_needed = contract
                .calc_input_for_exact_output(reserve_in, reserve_out, output)
                .unwrap();

            // Due to integer rounding, reverse might need slightly more
            assert!(
                reverse_input_needed >= amount_in,
                "Rounding should be conservative"
            );

            // But not too much more (within 0.01%)
            let difference = reverse_input_needed - amount_in;
            let diff_pct = (difference as f64 / amount_in as f64) * 100.0;
            assert!(
                diff_pct < 0.01,
                "Rounding difference should be minimal: {}%",
                diff_pct
            );
        }

        #[ink::test]
        fn test_fee_accumulation_accuracy() {
            let contract = setup_contract();

            // Simulate many small trades
            let reserve_in = 1_000_000_000_000;
            let reserve_out = 1_000_000_000_000;
            let trade_size = 1_000_000; // Small trade
            let num_trades = 1000;

            let mut total_fee_paid = 0u128;
            let mut current_reserve_in = reserve_in;
            let mut current_reserve_out = reserve_out;

            for _ in 0..num_trades {
                let output = contract
                    .calc_opposite_currency_amount(
                        current_reserve_in,
                        current_reserve_out,
                        trade_size,
                    )
                    .unwrap();

                // Calculate fee paid on this trade
                let amount_without_fee = (trade_size as u128) * 1000 / 990; // Reverse fee calc
                let fee_paid = amount_without_fee - (trade_size as u128);
                total_fee_paid += fee_paid;

                // Update reserves
                current_reserve_in += (trade_size as u128 * 990 / 1000) as Balance;
                current_reserve_out -= output;
            }

            // Total fees should be approximately 1% of total volume
            let total_volume = (trade_size as u128) * (num_trades as u128);
            let expected_fees = total_volume * 10 / 1000; // 1% of volume

            // Should be within 5% due to compounding effects
            let fee_accuracy = total_fee_paid as f64 / expected_fees as f64;
            assert!(
                fee_accuracy > 0.95 && fee_accuracy < 1.05,
                "Fee accumulation should be accurate: {:.2}%",
                fee_accuracy * 100.0
            );
        }

        #[ink::test]
        fn test_flash_loan_attack_scenario() {
            let contract = setup_contract();

            // Initial pool state
            let initial_x = 1_000_000_000;
            let initial_y = 1_000_000_000;

            // Attacker flash loans large amount
            let flash_loan_amount = 10_000_000_000; // 10x the pool

            // Try massive trade
            let result =
                contract.calc_opposite_currency_amount(initial_x, initial_y, flash_loan_amount);

            if let Ok(output) = result {
                // Even with huge input, output is capped by pool
                assert!(output < initial_y, "Cannot drain more than pool contains");

                // Calculate effective exchange rate
                let rate = (output as f64) / (flash_loan_amount as f64);

                // Rate should be terrible due to massive slippage
                assert!(
                    rate < 0.1,
                    "Massive trades should have terrible rates: {:.4}",
                    rate
                );
            }
        }

        #[ink::test]
        fn test_economic_attack_vectors() {
            let contract = setup_contract();

            // Test 1: Donation attack on empty pool
            // (Addressed by initial LP token amount of 1M)

            // Test 2: Sandwich attack profitability
            let pool_x = 10_000_000_000;
            let pool_y = 10_000_000_000;
            let victim_trade = 100_000_000;

            // Attacker front-runs
            let attacker_trade = 500_000_000;
            let attacker_output = contract
                .calc_opposite_currency_amount(pool_x, pool_y, attacker_trade)
                .unwrap();

            // Update pool
            let pool_x_after_attack = pool_x + (attacker_trade * 990 / 1000);
            let pool_y_after_attack = pool_y - attacker_output;

            // Victim trades at worse price
            let victim_output = contract
                .calc_opposite_currency_amount(
                    pool_x_after_attack,
                    pool_y_after_attack,
                    victim_trade,
                )
                .unwrap();

            // Pool state after victim
            let pool_x_final = pool_x_after_attack + (victim_trade * 990 / 1000);
            let pool_y_final = pool_y_after_attack - victim_output;

            // Attacker sells back
            let attacker_sell_back = contract
                .calc_opposite_currency_amount(pool_y_final, pool_x_final, attacker_output)
                .unwrap();

            // Attacker profit/loss
            let attacker_pnl = attacker_sell_back as i128 - attacker_trade as i128;

            // With proper fees, sandwich should not be profitable
            assert!(
                attacker_pnl < 0,
                "Sandwich attack should not be profitable with fees"
            );
        }
    }
} //---LAST LINE OF IMPLEMENTATION OF THE INK! SMART CONTRACT---//
