#![cfg_attr(not(feature = "std"), no_std)]

use ink::prelude::{collections::BTreeMap, string::String, vec::Vec};
use ink::primitives::AccountId;
use scale::{Decode, Encode};

// ==============================
// Macros (exported at crate root)
// ==============================

/// Macro to implement basic router functionality
#[macro_export]
macro_rules! impl_prism_router {
    ($contract:ty) => {
        impl $crate::prism::PrismRouter for $contract {
            fn create_context(&self) -> $crate::prism::CallContext {
                $crate::prism::CallContext::new(
                    self.env().caller(),
                    self.env().account_id(),
                    self.env().block_timestamp(),
                )
            }

            fn find_route(
                &self,
                selector: [u8; 4],
            ) -> Result<ink::primitives::AccountId, $crate::prism::PrismError> {
                self.router_state
                    .get_route(selector)
                    .map(|route| route.logic)
            }

            fn register_route(
                &mut self,
                selector: [u8; 4],
                logic: ink::primitives::AccountId,
            ) -> Result<(), $crate::prism::PrismError> {
                self.router_state.add_route(selector, logic, 300_000) // 5 min default
            }

            fn update_route(
                &mut self,
                selector: [u8; 4],
                new_logic: ink::primitives::AccountId,
            ) -> Result<(), $crate::prism::PrismError> {
                self.router_state
                    .routes
                    .get_mut(&selector)
                    .ok_or($crate::prism::PrismError::RouteNotFound)
                    .map(|route| route.logic = new_logic)
            }

            fn check_route_active(
                &self,
                selector: [u8; 4],
            ) -> Result<(), $crate::prism::PrismError> {
                self.router_state
                    .routes
                    .get(&selector)
                    .ok_or($crate::prism::PrismError::RouteNotFound)
                    .and_then(|route| {
                        if route.active {
                            Ok(())
                        } else {
                            Err($crate::prism::PrismError::InactiveRoute)
                        }
                    })
            }

            fn activate_route(
                &mut self,
                selector: [u8; 4],
            ) -> Result<(), $crate::prism::PrismError> {
                self.router_state
                    .routes
                    .get_mut(&selector)
                    .ok_or($crate::prism::PrismError::RouteNotFound)
                    .map(|route| route.active = true)
            }

            fn deactivate_route(
                &mut self,
                selector: [u8; 4],
            ) -> Result<(), $crate::prism::PrismError> {
                self.router_state
                    .routes
                    .get_mut(&selector)
                    .ok_or($crate::prism::PrismError::RouteNotFound)
                    .map(|route| route.active = false)
            }
        }
    };
}

#[macro_export]
macro_rules! prism_call {
    ($storage:expr, $method:literal, $returns:ty $(, $arg:expr)*) => {{
        use ink::env::call::{build_call, ExecutionInput, Selector};
        use ink::env::hash::{Blake2x256, HashOutput};

        // Calculate selector at runtime
        let mut output = <Blake2x256 as HashOutput>::Type::default();
        ink::env::hash_bytes::<Blake2x256>($method.as_bytes(), &mut output);
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&output[0..4]);

        // Build execution input with all arguments
        let exec_input = ExecutionInput::new(Selector::new(selector))
            $(.push_arg($arg))*;

        // Use try_invoke for better error handling
        build_call::<ink::env::DefaultEnvironment>()
            .call($storage)
            .gas_limit(0)
            .exec_input(exec_input)
            .returns::<$returns>()
            .try_invoke()
    }};
}

/// Prism Pattern Library - Core components for building Prism architecture contracts
pub mod prism {
    use super::*;
    use d9_environment;

    // ==============================
    // Core Types
    // ==============================

    /// Call context passed through the prism
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct CallContext {
        /// Original caller (end user)
        pub origin: AccountId,
        /// Router that created this context
        pub router: AccountId,
        /// Timestamp of the call
        pub timestamp: Timestamp,
        /// Call path for debugging
        pub path: Vec<AccountId>,
    }

    impl CallContext {
        pub fn new(origin: AccountId, router: AccountId, timestamp: Timestamp) -> Self {
            let mut path = Vec::new();
            path.push(router);
            Self {
                origin,
                router,
                timestamp,
                path,
            }
        }

        /// Add a contract to the call path
        pub fn add_to_path(&mut self, contract: AccountId) {
            self.path.push(contract);
        }

        /// Verify context is valid and recent
        pub fn verify(
            &self,
            current_time: Timestamp,
            max_age: Timestamp,
        ) -> Result<(), PrismError> {
            if current_time > self.timestamp + max_age {
                return Err(PrismError::ContextExpired);
            }

            Ok(())
        }
    }

    /// Storage access token for controlled access
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct StorageAccessToken {
        /// Logic contract making the request
        pub accessor: AccountId,
        /// Operation being performed
        pub operation: StorageOperation,
        /// Expiry time
        pub expires_at: Timestamp,
        /// Single use token
        pub nonce: u64,
    }

    /// Types of storage operations
    #[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum StorageOperation {
        Read,
        Write,
        Increment,
        Decrement,
        Admin,
    }

    /// Logic contract capability declaration
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct LogicCapability {
        /// Function selectors this logic handles
        pub selectors: Vec<[u8; 4]>,
        /// Version of this logic
        pub version: u32,
    }

    /// Route information
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Route {
        /// Function selector
        pub selector: [u8; 4],
        /// Logic contract handling this route
        pub logic: AccountId,
        /// Is this route active
        pub active: bool,
        /// Minimum context age allowed
        pub max_context_age: Timestamp,
    }

    // ==============================
    // Errors
    // ==============================

    #[derive(Debug, PartialEq, Eq, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum PrismError {
        // Context errors
        ContextExpired,
        InvalidContext,
        UnauthorizedRouter,

        // Storage errors
        UnauthorizedAccess,
        InvalidToken,
        TokenExpired,
        TokenAlreadyUsed,

        // Routing errors
        RouteNotFound,
        LogicNotFound,
        InactiveRoute,

        // Extension errors
        MissingRequiredExtension,
        ExtensionNotFound,

        // General errors
        NotImplemented,
        InvalidOperation,
    }

    // ==============================
    // Traits
    // ==============================

    /// Trait for Prism Routers
    pub trait PrismRouter {
        /// Create a new call context
        fn create_context(&self, nonce: u64) -> CallContext;

        /// Find logic contract for selector
        fn find_route(&self, selector: [u8; 4]) -> Result<AccountId, PrismError>;

        /// Register new route
        fn register_route(&mut self, selector: [u8; 4], logic: AccountId)
            -> Result<(), PrismError>;

        /// Update existing route
        fn update_route(
            &mut self,
            selector: [u8; 4],
            new_logic: AccountId,
        ) -> Result<(), PrismError>;

        fn activate_route(&mut self, selector: [u8; 4]) -> Result<(), PrismError>;

        /// Deactivate route
        fn deactivate_route(&mut self, selector: [u8; 4]) -> Result<(), PrismError>;
    }

    /// Trait for Storage Cores
    pub trait PrismStorage {
        /// Verify storage access token
        fn verify_token(&self, token: &StorageAccessToken) -> Result<(), PrismError>;

        /// Register authorized logic contract
        fn authorize_logic(&mut self, logic: AccountId) -> Result<(), PrismError>;

        /// Revoke logic authorization
        fn revoke_logic(&mut self, logic: AccountId) -> Result<(), PrismError>;

        /// Check if logic is authorized
        fn is_authorized(&self, logic: AccountId) -> bool;
    }

    /// Trait for Logic Contracts
    pub trait PrismLogic {
        /// Get capability declaration
        fn get_capabilities(&self) -> LogicCapability;

        /// Initialize with storage addresses
        fn initialize_storage(
            &mut self,
            core: AccountId,
            extensions: Vec<(String, AccountId)>,
        ) -> Result<(), PrismError>;

        /// Verify call context
        fn verify_context(&self, context: &CallContext) -> Result<(), PrismError>;
    }

    /// Trait for Storage Extensions
    pub trait PrismExtension {
        /// Get extension name
        fn extension_name(&self) -> String;

        /// Get extension version
        fn extension_version(&self) -> u32;

        /// Verify accessor is authorized
        fn verify_access(&self, accessor: AccountId) -> Result<(), PrismError>;
    }

    // ==============================
    // Helper Implementations
    // ==============================

    /// Basic router state management
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
    )]
    pub struct RouterState {
        /// Current routes
        pub routes: BTreeMap<[u8; 4], Route>,
        /// Authorized routers (for multi-router setups)
        pub authorized_routers: Vec<AccountId>,
        /// Nonce counter
        pub nonce_counter: u64,
    }

    impl RouterState {
        pub fn new() -> Self {
            Self {
                routes: BTreeMap::new(),
                authorized_routers: Vec::new(),
                nonce_counter: 0,
            }
        }

        pub fn next_nonce(&mut self) -> u64 {
            self.nonce_counter += 1;
            self.nonce_counter
        }

        pub fn add_route(
            &mut self,
            selector: [u8; 4],
            logic: AccountId,
            max_age: Timestamp,
        ) -> Result<(), PrismError> {
            self.routes.insert(
                selector,
                Route {
                    selector,
                    logic,
                    active: true,
                    max_context_age: max_age,
                },
            );
            Ok(())
        }

        pub fn get_route(&self, selector: [u8; 4]) -> Result<&Route, PrismError> {
            self.routes
                .get(&selector)
                .ok_or(PrismError::RouteNotFound)
                .and_then(|route| {
                    if route.active {
                        Ok(route)
                    } else {
                        Err(PrismError::InactiveRoute)
                    }
                })
        }
    }

    /// Basic storage authorization management
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
    )]
    pub struct StorageAuth {
        /// Authorized logic contracts
        pub authorized_logic: Vec<AccountId>,
        /// Used tokens (prevent replay)
        pub used_tokens: Vec<u64>,
        /// Token counter
        pub token_counter: u64,
    }

    impl StorageAuth {
        pub fn new() -> Self {
            Self {
                authorized_logic: Vec::new(),
                used_tokens: Vec::new(),
                token_counter: 0,
            }
        }

        pub fn is_authorized(&self, logic: AccountId) -> bool {
            self.authorized_logic.contains(&logic)
        }

        pub fn authorize(&mut self, logic: AccountId) -> Result<(), PrismError> {
            if !self.is_authorized(logic) {
                self.authorized_logic.push(logic);
            }
            Ok(())
        }

        pub fn create_token(
            &mut self,
            accessor: AccountId,
            operation: StorageOperation,
            duration: Timestamp,
            current_time: Timestamp,
        ) -> StorageAccessToken {
            self.token_counter += 1;
            StorageAccessToken {
                accessor,
                operation,
                expires_at: current_time + duration,
                nonce: self.token_counter,
            }
        }

        pub fn verify_token(
            &mut self,
            token: &StorageAccessToken,
            current_time: Timestamp,
        ) -> Result<(), PrismError> {
            // Check if used
            if self.used_tokens.contains(&token.nonce) {
                return Err(PrismError::TokenAlreadyUsed);
            }

            // Check expiry
            if current_time > token.expires_at {
                return Err(PrismError::TokenExpired);
            }

            // Check authorization
            if !self.is_authorized(token.accessor) {
                return Err(PrismError::UnauthorizedAccess);
            }

            // Mark as used
            self.used_tokens.push(token.nonce);

            // Clean old tokens periodically
            if self.used_tokens.len() > 1000 {
                self.used_tokens.drain(0..500);
            }

            Ok(())
        }
    }

    /// Extension registry for logic contracts
    #[derive(Debug, Clone, Encode, Decode)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
    )]
    pub struct ExtensionRegistry {
        /// Core storage address
        pub storage_core: AccountId,
        /// Extension name -> address mapping
        pub extensions: BTreeMap<String, AccountId>,
    }

    impl ExtensionRegistry {
        pub fn new(storage_core: AccountId) -> Self {
            Self {
                storage_core,
                extensions: BTreeMap::new(),
            }
        }

        pub fn register_extension(&mut self, name: String, address: AccountId) {
            self.extensions.insert(name, address);
        }

        pub fn get_extension(&self, name: &str) -> Option<AccountId> {
            self.extensions.get(name).copied()
        }

        pub fn get_all_extensions(&self) -> Vec<(String, AccountId)> {
            self.extensions
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        }
    }

    /// Type alias for timestamp from D9Environment
    pub type Timestamp = <d9_environment::D9Environment as ink::env::Environment>::Timestamp;

    /// Type alias for balance from D9Environment
    pub type Balance = <d9_environment::D9Environment as ink::env::Environment>::Balance;
}

// Re-export everything from the prism module
pub use prism::*;
