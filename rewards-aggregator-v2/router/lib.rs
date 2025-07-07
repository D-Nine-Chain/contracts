#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod rewards_aggregator_router {
    use prism::{CallContext, RouterState, LogicCapability, PrismError};
    use prism::prism_call;
    use safety::{AdminControl, Pausable, ReentrancyGuard, SafetyError, PauseReason};
    use ink::prelude::vec::Vec;
    
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        OnlyCallableBy(AccountId),
        SafetyError(SafetyError),
        PrismError(PrismError),
        StorageError,
        LogicError,
        EnvironmentError,
        AlreadyInitialized,
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
    
    #[ink(event)]
    pub struct LogicRegistered {
        #[ink(topic)]
        logic: AccountId,
        version: u32,
        selectors: Vec<[u8; 4]>,
    }
    
    #[ink(storage)]
    pub struct RewardsAggregatorRouter {
        admin: AdminControl,
        pausable: Pausable,
        reentrancy_guard: ReentrancyGuard,
        router_state: RouterState,
        storage_core: AccountId,
        node_reward_contract: AccountId,
        merchant_contract: AccountId,
        amm_contract: AccountId,
    }
    
    impl RewardsAggregatorRouter {
        #[ink(constructor)]
        pub fn new(
            storage_core: AccountId,
            node_reward_contract: AccountId,
            merchant_contract: AccountId,
            amm_contract: AccountId,
        ) -> Self {
            Self {
                admin: AdminControl::new(Self::env().caller()),
                pausable: Pausable::new(),
                reentrancy_guard: ReentrancyGuard::new(),
                router_state: RouterState::new(),
                storage_core,
                node_reward_contract,
                merchant_contract,
                amm_contract,
            }
        }
        
        // Admin functions
        #[ink(message)]
        pub fn pause(&mut self) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.pausable.pause(self.env().block_timestamp(), PauseReason::Maintenance)?;
            Ok(())
        }
        
        #[ink(message)]
        pub fn unpause(&mut self) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.pausable.unpause()?;
            Ok(())
        }
        
        #[ink(message)]
        pub fn transfer_admin(&mut self, new_admin: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.admin.propose_admin(self.env().caller(), new_admin)?;
            Ok(())
        }
        
        #[ink(message)]
        pub fn accept_admin(&mut self) -> Result<(), Error> {
            self.admin.accept_admin(self.env().caller(), self.env().block_timestamp())?;
            Ok(())
        }
        
        #[ink(message)]
        pub fn activate_route(&mut self, selector: [u8; 4]) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.router_state.routes.get_mut(&selector)
                .ok_or(PrismError::RouteNotFound)?
                .active = true;
            Ok(())
        }
        
        #[ink(message)]
        pub fn register_logic_contract(&mut self, logic: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            
            // Get capabilities from logic contract
            let capabilities: LogicCapability = prism_call!(
                logic,
                "get_capabilities",
                LogicCapability
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Register all selectors
            for selector in capabilities.selectors.iter() {
                self.router_state.add_route(*selector, logic, 300_000)?;
            }
            
            // Authorize logic in storage
            prism_call!(
                self.storage_core,
                "authorize_logic",
                Result<(), Error>,
                logic
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Initialize the logic contract
            prism_call!(
                logic,
                "initialize_storage",
                Result<(), Error>,
                self.storage_core,
                Vec::<(ink::prelude::string::String, AccountId)>::new()
            ).map_err(|_| Error::EnvironmentError)??;
            
            // Pass AMM contract to merchant logic if it handles merchant operations
            if capabilities.selectors.contains(&ink::selector_bytes!("process_merchant_payment")) {
                prism_call!(
                    logic,
                    "set_amm_contract",
                    Result<(), Error>,
                    self.amm_contract
                ).map_err(|_| Error::EnvironmentError)??;
            }
            
            self.env().emit_event(LogicRegistered {
                logic,
                version: capabilities.version,
                selectors: capabilities.selectors,
            });
            
            Ok(())
        }
        
        // Routing functions
        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, session_index: u32) -> Result<Balance, Error> {
            self.only_callable_by(self.env().caller(), self.node_reward_contract)?;
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let selector = ink::selector_bytes!("update_pool_and_retrieve");
            let logic = self.router_state.get_route(selector)?.logic;
            let context = self.create_context();
            
            let result: Result<Balance, Error> = prism_call!(
                logic,
                "update_pool_and_retrieve",
                Result<Balance, Error>,
                context,
                session_index
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            result
        }
        
        #[ink(message)]
        pub fn pay_node_reward(&mut self, account_id: AccountId, amount: Balance) -> Result<(), Error> {
            self.only_callable_by(self.env().caller(), self.node_reward_contract)?;
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let selector = ink::selector_bytes!("pay_node_reward");
            let logic = self.router_state.get_route(selector)?.logic;
            let context = self.create_context();
            
            prism_call!(
                logic,
                "pay_node_reward",
                Result<(), Error>,
                context,
                account_id,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            Ok(())
        }
        
        #[ink(message)]
        pub fn deduct_from_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            self.only_callable_by(self.env().caller(), self.node_reward_contract)?;
            self.pausable.ensure_not_paused()?;
            
            let selector = ink::selector_bytes!("deduct_from_reward_pool");
            let logic = self.router_state.get_route(selector)?.logic;
            let context = self.create_context();
            
            prism_call!(
                logic,
                "deduct_from_reward_pool",
                Result<(), Error>,
                context,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            Ok(())
        }
        
        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self, merchant_id: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.env().caller(), self.merchant_contract)?;
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let selector = ink::selector_bytes!("process_merchant_payment");
            let logic = self.router_state.get_route(selector)?.logic;
            let amount = self.env().transferred_value();
            let context = self.create_context();
            
            prism_call!(
                logic,
                "process_merchant_payment",
                Result<(), Error>,
                context,
                merchant_id,
                amount
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            Ok(())
        }
        
        #[ink(message)]
        pub fn merchant_user_redeem_d9(&mut self, user_account: AccountId, redeemable_usdt: Balance) -> Result<Balance, Error> {
            self.only_callable_by(self.env().caller(), self.merchant_contract)?;
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let selector = ink::selector_bytes!("merchant_user_redeem_d9");
            let logic = self.router_state.get_route(selector)?.logic;
            let context = self.create_context();
            
            let result: Result<Balance, Error> = prism_call!(
                logic,
                "merchant_user_redeem_d9",
                Result<Balance, Error>,
                context,
                user_account,
                redeemable_usdt
            ).map_err(|_| Error::EnvironmentError)??;
            
            self.reentrancy_guard.exit();
            result
        }
        
        // Getters
        #[ink(message)]
        pub fn get_storage_core(&self) -> AccountId {
            self.storage_core
        }
        
        #[ink(message)]
        pub fn get_admin(&self) -> AccountId {
            self.admin.get_admin()
        }
        
        #[ink(message)]
        pub fn is_paused(&self) -> bool {
            self.pausable.is_paused()
        }
        
        // Helper functions
        fn create_context(&mut self) -> CallContext {
            let nonce = self.router_state.next_nonce();
            CallContext::new(
                self.env().caller(),
                self.env().account_id(),
                self.env().block_timestamp(),
                nonce,
            )
        }
        
        fn only_callable_by(&self, caller: AccountId, expected: AccountId) -> Result<(), Error> {
            if caller != expected {
                return Err(Error::OnlyCallableBy(expected));
            }
            Ok(())
        }
    }
    
    // Router functionality is implemented through create_context method directly
    
}