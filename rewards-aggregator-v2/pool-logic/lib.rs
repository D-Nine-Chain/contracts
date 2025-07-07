#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract(env = chain_extension::D9Environment)]
mod pool_operations_logic {
    use prism::{CallContext, ExtensionRegistry, PrismLogic, LogicCapability, PrismError};
    use prism::prism_call;
    use safety::{ReentrancyGuard, SafetyError};
    use ink::prelude::{vec::Vec, string::String};
    use sp_arithmetic::Perquintill;
    
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        UnauthorizedRouter,
        InvalidContext,
        AlreadyInitialized,
        SessionPoolNotReady,
        StorageError,
        EnvironmentError,
        TransferFailed,
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
    pub struct PoolOperationsLogic {
        reentrancy_guard: ReentrancyGuard,
        authorized_routers: Vec<AccountId>,
        storage_core: AccountId,
        extension_registry: ExtensionRegistry,
        version: u32,
    }
    
    impl PoolOperationsLogic {
        #[ink(constructor)]
        pub fn new() -> Self {
            Self {
                reentrancy_guard: ReentrancyGuard::new(),
                authorized_routers: Vec::new(),
                storage_core: AccountId::from([0u8; 32]),
                extension_registry: ExtensionRegistry::new(AccountId::from([0u8; 32])),
                version: 1,
            }
        }
        
        #[ink(message)]
        pub fn get_capabilities(&self) -> LogicCapability {
            LogicCapability {
                selectors: vec![
                    ink::selector_bytes!("update_pool_and_retrieve"),
                    ink::selector_bytes!("pay_node_reward"),
                    ink::selector_bytes!("deduct_from_reward_pool"),
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
            
            for (name, address) in extensions {
                self.extension_registry.register_extension(name, address);
            }
            
            self.authorized_routers.push(self.env().caller());
            
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
        
        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, context: CallContext, session_index: u32) -> Result<Balance, Error> {
            self.verify_context(context)?;
            self.reentrancy_guard.enter()?;
            
            // Get current last session
            let last_session = prism_call!(
                self.storage_core,
                "get_last_session",
                u32
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Get total volume
            let total_volume = prism_call!(
                self.storage_core,
                "get_total_volume",
                Balance
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Store session volume
            prism_call!(
                self.storage_core,
                "set_session_volume",
                Result<(), Error>,
                session_index,
                total_volume
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Calculate session delta
            let session_delta = self.calculate_session_delta(session_index, last_session, total_volume)?;
            
            // Calculate 3% of delta
            let three_percent = Perquintill::from_percent(3);
            let three_percent_of_delta = three_percent.mul_floor(session_delta);
            
            // Update reward pool
            prism_call!(
                self.storage_core,
                "update_reward_pool",
                Result<(), Error>,
                three_percent_of_delta
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Get current pool balance
            let current_pool = prism_call!(
                self.storage_core,
                "get_total_reward_pool",
                Balance
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Calculate 10% for distribution
            let ten_percent = Perquintill::from_percent(10);
            let reward_pool = ten_percent.mul_floor(current_pool);
            
            // Update last session
            prism_call!(
                self.storage_core,
                "set_last_session",
                Result<(), Error>,
                session_index
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            Ok(reward_pool)
        }
        
        #[ink(message)]
        pub fn pay_node_reward(&mut self, context: CallContext, account_id: AccountId, amount: Balance) -> Result<(), Error> {
            self.verify_context(context)?;
            self.reentrancy_guard.enter()?;
            
            // Transfer funds
            self.env().transfer(account_id, amount)
                .map_err(|_| Error::TransferFailed)?;
            
            // Update storage
            prism_call!(
                self.storage_core,
                "subtract_from_reward_pool",
                Result<(), Error>,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            Ok(())
        }
        
        #[ink(message)]
        pub fn deduct_from_reward_pool(&mut self, context: CallContext, amount: Balance) -> Result<(), Error> {
            self.verify_context(context)?;
            
            prism_call!(
                self.storage_core,
                "subtract_from_reward_pool",
                Result<(), Error>,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            Ok(())
        }
        
        fn calculate_session_delta(&self, session_index: u32, last_session: u32, total_volume: Balance) -> Result<Balance, Error> {
            if session_index <= last_session || last_session == 0 {
                return Ok(total_volume);
            }
            
            let last_volume = prism_call!(
                self.storage_core,
                "get_session_volume",
                Balance,
                last_session
            ).map_err(|_| Error::EnvironmentError)??;
            
            Ok(total_volume.saturating_sub(last_volume))
        }
    }
    
    impl PrismLogic for PoolOperationsLogic {
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