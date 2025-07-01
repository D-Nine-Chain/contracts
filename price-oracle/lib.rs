#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod d9_price_oracle {
    use super::*;
    use ink::env::call::{build_call, ExecutionInput, Selector};
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{Decode, Encode};

    #[ink(storage)]
    pub struct D9PriceOracle {
        /// Admin account
        admin: AccountId,
        /// AMM contract for price queries
        amm_contract: AccountId,
        /// Highest recorded price (USDT per D9 with precision)
        highest_price: Balance,
        /// Timestamp of highest price
        highest_price_timestamp: Timestamp,
        /// Price precision factor
        precision: Balance,
        /// Default protection threshold (90%)
        default_threshold: u32,
        /// Whether oracle is active
        is_active: bool,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct PriceInfo {
        pub current_price: Balance,
        pub highest_price: Balance,
        pub protected_price: Balance,
        pub protection_active: bool,
        pub timestamp: Timestamp,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        NotAdmin,
        OracleNotActive,
        FailedToGetReserves,
        DivisionByZero,
        InvalidThreshold,
    }

    #[ink(event)]
    pub struct NewHighestPrice {
        #[ink(topic)]
        old_price: Balance,
        #[ink(topic)]
        new_price: Balance,
        timestamp: Timestamp,
    }

    #[ink(event)]
    pub struct PriceProtectionTriggered {
        current_price: Balance,
        protected_price: Balance,
        threshold_used: u32,
    }

    impl D9PriceOracle {
        #[ink(constructor)]
        pub fn new(amm_contract: AccountId) -> Self {
            Self {
                admin: Self::env().caller(),
                amm_contract,
                highest_price: 0,
                highest_price_timestamp: 0,
                precision: 1_000_000,  // 6 decimal places
                default_threshold: 90, // 90%
                is_active: true,
            }
        }

        /// Get protected price information
        #[ink(message)]
        pub fn get_protected_price(&mut self) -> Result<PriceInfo, Error> {
            self.get_protected_price_with_threshold(self.default_threshold)
        }

        /// Get protected price with custom threshold
        #[ink(message)]
        pub fn get_protected_price_with_threshold(
            &mut self,
            threshold_percent: u32,
        ) -> Result<PriceInfo, Error> {
            if !self.is_active {
                return Err(Error::OracleNotActive);
            }

            if threshold_percent > 100 {
                return Err(Error::InvalidThreshold);
            }

            // Get current price from AMM
            let current_price = self.fetch_current_price_from_amm()?;

            // Update highest if needed
            if current_price > self.highest_price {
                let old_price = self.highest_price;
                self.highest_price = current_price;
                self.highest_price_timestamp = self.env().block_timestamp();

                self.env().emit_event(NewHighestPrice {
                    old_price,
                    new_price: current_price,
                    timestamp: self.env().block_timestamp(),
                });
            }

            // Calculate protected price
            let min_acceptable_price = self
                .highest_price
                .saturating_mul(threshold_percent as Balance)
                .saturating_div(100);

            let protected_price = current_price.max(min_acceptable_price);
            let protection_active = protected_price > current_price;

            if protection_active {
                self.env().emit_event(PriceProtectionTriggered {
                    current_price,
                    protected_price,
                    threshold_used: threshold_percent,
                });
            }

            Ok(PriceInfo {
                current_price,
                highest_price: self.highest_price,
                protected_price,
                protection_active,
                timestamp: self.env().block_timestamp(),
            })
        }

        /// Calculate D9 amount using protected price
        #[ink(message)]
        pub fn calculate_d9_amount_protected(
            &mut self,
            usdt_amount: Balance,
        ) -> Result<(Balance, PriceInfo), Error> {
            let price_info = self.get_protected_price()?;

            // Calculate D9 amount using protected price
            // D9 = USDT * precision / price
            let d9_amount = usdt_amount
                .saturating_mul(self.precision)
                .checked_div(price_info.protected_price)
                .ok_or(Error::DivisionByZero)?;

            Ok((d9_amount, price_info))
        }

        /// Get just the current price without updating highest
        #[ink(message)]
        pub fn get_current_price(&self) -> Result<Balance, Error> {
            if !self.is_active {
                return Err(Error::OracleNotActive);
            }
            self.fetch_current_price_from_amm()
        }

        /// Fetch current price from AMM contract
        fn fetch_current_price_from_amm(&self) -> Result<Balance, Error> {
            // Get reserves from AMM
            let reserves_result = build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(ExecutionInput::new(Selector::new(selector_bytes!(
                    "get_currency_reserves"
                ))))
                .returns::<(Balance, Balance)>()
                .try_invoke();

            let (d9_reserves, usdt_reserves) = reserves_result
                .map_err(|_| Error::FailedToGetReserves)?
                .map_err(|_| Error::FailedToGetReserves)?;

            // Calculate price as USDT per D9 with precision
            let price = usdt_reserves
                .saturating_mul(self.precision)
                .checked_div(d9_reserves)
                .ok_or(Error::DivisionByZero)?;

            Ok(price)
        }

        // Admin functions

        #[ink(message)]
        pub fn initialize_highest_price(&mut self) -> Result<(), Error> {
            self.only_admin()?;

            if self.highest_price == 0 {
                let current_price = self.fetch_current_price_from_amm()?;
                self.highest_price = current_price;
                self.highest_price_timestamp = self.env().block_timestamp();
            }

            Ok(())
        }

        #[ink(message)]
        pub fn set_active(&mut self, active: bool) -> Result<(), Error> {
            self.only_admin()?;
            self.is_active = active;
            Ok(())
        }

        #[ink(message)]
        pub fn set_default_threshold(&mut self, threshold: u32) -> Result<(), Error> {
            self.only_admin()?;
            if threshold > 100 {
                return Err(Error::InvalidThreshold);
            }
            self.default_threshold = threshold;
            Ok(())
        }

        #[ink(message)]
        pub fn reset_highest_price(&mut self) -> Result<(), Error> {
            self.only_admin()?;
            self.highest_price = 0;
            self.highest_price_timestamp = 0;
            Ok(())
        }

        #[ink(message)]
        pub fn update_amm_contract(&mut self, new_amm: AccountId) -> Result<(), Error> {
            self.only_admin()?;
            self.amm_contract = new_amm;
            Ok(())
        }

        #[ink(message)]
        pub fn change_admin(&mut self, new_admin: AccountId) -> Result<(), Error> {
            self.only_admin()?;
            self.admin = new_admin;
            Ok(())
        }

        // View functions

        #[ink(message)]
        pub fn get_oracle_info(&self) -> (AccountId, Balance, Timestamp, u32, bool) {
            (
                self.amm_contract,
                self.highest_price,
                self.highest_price_timestamp,
                self.default_threshold,
                self.is_active,
            )
        }

        fn only_admin(&self) -> Result<(), Error> {
            if self.env().caller() != self.admin {
                return Err(Error::NotAdmin);
            }
            Ok(())
        }
    }
}
