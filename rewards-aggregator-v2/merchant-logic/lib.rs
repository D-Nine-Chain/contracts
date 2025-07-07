#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract(env = chain_extension::D9Environment)]
mod merchant_operations_logic {
    use prism::{CallContext, ExtensionRegistry, PrismLogic, LogicCapability, PrismError};
    use prism::prism_call;
    use safety::{ReentrancyGuard, SafetyError};
    use ink::prelude::{vec::Vec, string::String};
    use scale::{Decode, Encode};
    
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Currency {
        D9,
        Usdt,
    }
    
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Direction(Currency, Currency);
    
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        UnauthorizedRouter,
        InvalidContext,
        AlreadyInitialized,
        RedeemableUSDTZero,
        AddingVotes,
        TransferFailed,
        StorageError,
        EnvironmentError,
        SafetyError(SafetyError),
        PrismError(PrismError),
    }
    
    impl From<SafetyError> for Error {
        fn from(e: SafetyError) -> Self {
            Error::SafetyError(e)
        }
    }
    
    impl From<PrismError> for Error {
        fn from(e: PrismError) -> Self {
            Error::PrismError(e)
        }
    }
    
    impl From<ink::LangError> for Error {
        fn from(_: ink::LangError) -> Self {
            Error::EnvironmentError
        }
    }
    
    #[ink(storage)]
    pub struct MerchantOperationsLogic {
        reentrancy_guard: ReentrancyGuard,
        authorized_routers: Vec<AccountId>,
        storage_core: AccountId,
        amm_contract: AccountId,
        extension_registry: ExtensionRegistry,
        version: u32,
    }
    
    const PRICE_PRECISION: Balance = 1_000_000;
    const PERCENT_PROTECT: Balance = 70;
    
    impl MerchantOperationsLogic {
        #[ink(constructor)]
        pub fn new() -> Self {
            Self {
                reentrancy_guard: ReentrancyGuard::new(),
                authorized_routers: Vec::new(),
                storage_core: AccountId::from([0u8; 32]),
                amm_contract: AccountId::from([0u8; 32]),
                extension_registry: ExtensionRegistry::new(AccountId::from([0u8; 32])),
                version: 1,
            }
        }
        
        #[ink(message)]
        pub fn get_capabilities(&self) -> LogicCapability {
            LogicCapability {
                selectors: vec![
                    ink::selector_bytes!("process_merchant_payment"),
                    ink::selector_bytes!("merchant_user_redeem_d9"),
                ],
                version: self.version,
            }
        }
        
        #[ink(message)]
        pub fn initialize_storage(
            &mut self,
            core: AccountId,
            extensions: Vec<(String, AccountId)>
        ) -> Result<(), Error> {
            if self.storage_core != AccountId::from([0u8; 32]) {
                return Err(Error::AlreadyInitialized);
            }
            
            self.storage_core = core;
            self.extension_registry = ExtensionRegistry::new(core);
            
            // Set AMM contract from extensions if provided
            for (name, address) in extensions {
                if name == "amm" {
                    self.amm_contract = address;
                }
                self.extension_registry.register_extension(name, address);
            }
            
            self.authorized_routers.push(self.env().caller());
            
            Ok(())
        }
        
        #[ink(message)]
        pub fn set_amm_contract(&mut self, amm: AccountId) -> Result<(), Error> {
            // Only router can set this
            if !self.authorized_routers.contains(&self.env().caller()) {
                return Err(Error::UnauthorizedRouter);
            }
            self.amm_contract = amm;
            Ok(())
        }
        
        #[ink(message)]
        pub fn verify_context(&self, context: CallContext) -> Result<(), Error> {
            if !self.authorized_routers.contains(&context.router) {
                return Err(Error::UnauthorizedRouter);
            }
            
            let current_time = self.env().block_timestamp();
            context.verify(current_time, 300_000)?;
            
            Ok(())
        }
        
        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self, context: CallContext, merchant_id: AccountId, amount: Balance) -> Result<(), Error> {
            self.verify_context(context)?;
            self.reentrancy_guard.enter()?;
            
            // Update merchant volume
            prism_call!(
                self.storage_core,
                "add_merchant_volume",
                Result<(), Error>,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Calculate and add votes
            let votes = self.calc_votes_from_d9(amount);
            self.env().extension().add_voting_interests(merchant_id, votes as u64)
                .map_err(|_| Error::AddingVotes)?;
            
            self.reentrancy_guard.exit();
            Ok(())
        }
        
        #[ink(message)]
        pub fn merchant_user_redeem_d9(&mut self, context: CallContext, user_account: AccountId, redeemable_usdt: Balance) -> Result<Balance, Error> {
            self.verify_context(context)?;
            self.reentrancy_guard.enter()?;
            
            if redeemable_usdt == 0 {
                return Err(Error::RedeemableUSDTZero);
            }
            
            // Get exchange rate from AMM
            let current_d9_amount = self.get_exchange_amount(
                Direction(Currency::Usdt, Currency::D9),
                redeemable_usdt
            )?;
            
            // Calculate current rate
            let current_rate = current_d9_amount
                .saturating_mul(PRICE_PRECISION)
                .saturating_div(redeemable_usdt);
            
            // Get/update highest rate
            let mut highest_rate = prism_call!(
                self.storage_core,
                "get_highest_price",
                Balance
            ).map_err(|_| Error::EnvironmentError)??;
            
            if highest_rate == 0 || current_rate > highest_rate {
                highest_rate = current_rate;
                prism_call!(
                    self.storage_core,
                    "set_highest_price",
                    Result<(), Error>,
                    highest_rate
                ).map_err(|_| Error::EnvironmentError)??;
            }
            
            // Calculate protected rate (70% of highest)
            let min_acceptable_rate = highest_rate
                .saturating_mul(PERCENT_PROTECT)
                .saturating_div(100);
            
            let effective_rate = current_rate.max(min_acceptable_rate);
            
            // Calculate final D9 amount
            let final_d9_amount = redeemable_usdt
                .saturating_mul(effective_rate)
                .saturating_div(PRICE_PRECISION);
            
            // Transfer D9 to user
            self.env().transfer(user_account, final_d9_amount)
                .map_err(|_| Error::TransferFailed)?;
            
            self.reentrancy_guard.exit();
            Ok(final_d9_amount)
        }
        
        fn get_exchange_amount(&self, direction: Direction, amount: Balance) -> Result<Balance, Error> {
            prism_call!(
                self.amm_contract,
                "get_exchange_amount",
                Result<Balance, Error>,
                direction,
                amount
            ).map_err(|_| Error::EnvironmentError)??
        }
        
        fn calc_votes_from_d9(&self, d9_amount: Balance) -> Balance {
            // 1 D9 = 1 vote for simplicity
            d9_amount
        }
    }
    
    impl PrismLogic for MerchantOperationsLogic {
        fn get_capabilities(&self) -> LogicCapability {
            self.get_capabilities()
        }
        
        fn initialize_storage(&mut self, core: AccountId, extensions: Vec<(String, AccountId)>) -> Result<(), PrismError> {
            self.initialize_storage(core, extensions)
                .map_err(|_| PrismError::NotImplemented)
        }
        
        fn verify_context(&self, context: &CallContext) -> Result<(), PrismError> {
            // Create a clone to pass by value to the internal method
            let context_clone = context.clone();
            self.verify_context(context_clone)
                .map_err(|_| PrismError::InvalidContext)
        }
    }
    
}