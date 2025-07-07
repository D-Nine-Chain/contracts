# The Prism Doctrine: A Modular Smart Contract Architecture for ink!

## Table of Contents

1. [Introduction](#introduction)
2. [Problem Statement](#problem-statement)
3. [Core Architecture](#core-architecture)
4. [Fundamental Rules](#fundamental-rules)
5. [Implementation Guide](#implementation-guide)
6. [Cross-Contract Communication](#cross-contract-communication)
7. [Security Model](#security-model)
8. [Best Practices](#best-practices)
9. [Code Templates](#code-templates)
10. [Migration & Upgrades](#migration--upgrades)

## Introduction

The Prism Doctrine is a smart contract architecture pattern designed
specifically for ink! that separates contracts into distinct layers: Router
(entry point), Storage (state), and Logic (business rules). This separation
enables upgradeability, modularity, and enhanced security while maintaining
consistency and auditability.

**Key Principle**: _"Gas is cheap, security and maintainability are expensive."_

**ink! Version**: This guide is for ink! 4.3.0 and later.

## Problem Statement

### Traditional Monolithic Contracts Fail Because:

1. **No Upgradeability** - Bugs become permanent, improvements impossible
2. **Growing Complexity** - Single contract becomes unmaintainable
3. **Security Challenges** - Large attack surface, difficult to audit
4. **Limited Modularity** - Can't share components between contracts
5. **State Migration Hell** - Upgrading requires complex state transfers

### The Prism Solution

Prism splits contracts into three layers:

- **Router**: Immutable entry point, handles routing and access control
- **Storage**: Holds all state, controls access, enables persistence
- **Logic**: Stateless business logic, easily upgradeable

```
External World
      ↓
   Router (immutable entry point)
   ↙    ↘
Logic    Storage
```

## Core Architecture

### 1. Router Contract

The router is the **single entry point** for all external interactions.

**Responsibilities:**

- Create and validate CallContext
- Route calls to appropriate logic contracts
- Enforce access control
- Manage pausing/emergency stops
- Emit events for monitoring

**What it MUST NOT do:**

- Contain business logic
- Store business state
- Make decisions beyond routing

### 2. Storage Core

The storage core is the **single source of truth** for all state.

**Responsibilities:**

- Store all contract state
- Validate access permissions via `env().caller()`
- Maintain state consistency
- Track authorized logic contracts
- Provide atomic operations

**Key Features:**

- Only logic contracts can write
- Anyone can read (if public)
- Supports modular extensions
- Maintains upgrade history
- All state mutations are `#[ink(message)]` functions

### 3. Logic Contracts

Logic contracts contain **pure business logic** with minimal state.

**Responsibilities:**

- Implement business rules
- Validate operations
- Calculate results
- Call storage via cross-contract calls
- Maintain computation integrity

**Constraints:**

- Nearly stateless (only caching/temporary state)
- Must validate CallContext
- Can only be called by authorized routers
- Cannot store critical business state
- Must use `build_call` for storage access

### 4. CallContext

The CallContext is a **cryptographically secure request tracker** that flows
through all calls.

```rust
pub struct CallContext {
    pub origin: AccountId,      // Original caller
    pub router: AccountId,      // Router that created context
    pub timestamp: Timestamp,   // When created
    pub nonce: u64,            // Prevent replay
    pub path: Vec<AccountId>,  // Call path for debugging
}
```

**Purpose:**

- Maintain caller identity through call chain
- Prevent replay attacks
- Enable audit trails
- Enforce timeouts

### 5. Storage Extensions

Modular storage components for **organized state management**.

**Examples:**

- PriceHistory extension
- UserBalances extension
- VotingPower extension
- Merchant data extension

## Fundamental Rules

### Rule 1: All External Calls Through Router

**NO EXCEPTIONS**. External contracts and users MUST only interact with the
router.

```rust
// ❌ WRONG - Direct call to logic
merchant_contract.call_logic_directly()

// ✅ CORRECT - Through router
merchant_contract.call_router() → router.call_logic()
```

### Rule 2: Context Flows Through Everything

Every call between contracts MUST include CallContext as the first parameter.

```rust
// ❌ WRONG - No context
#[ink(message)]
pub fn process_payment(&mut self, amount: Balance) -> Result<(), Error>

// ✅ CORRECT - With context
#[ink(message)]
pub fn process_payment(&mut self, context: CallContext, amount: Balance) -> Result<(), Error>
```

### Rule 3: Storage Access is Privileged

Storage contracts verify caller authorization using `env().caller()`.

```rust
#[ink(message)]
pub fn update_balance(&mut self, user: AccountId, amount: Balance) -> Result<(), Error> {
    // Storage gets caller from environment
    let caller = self.env().caller();
    
    // Verify caller is authorized logic
    if !self.authorized_logic.contains(&caller) {
        return Err(Error::UnauthorizedAccess);
    }
    
    // Update state
    self.balances.insert(user, &amount);
    Ok(())
}
```

### Rule 4: Logic Contracts are Stateless

Logic contracts MUST NOT store business state.

```rust
// ❌ WRONG - Storing business state
pub struct BadLogic {
    user_balances: Mapping<AccountId, Balance>, // NO!
}

// ✅ CORRECT - Only references
pub struct GoodLogic {
    storage_core: AccountId,        // Reference only
    authorized_routers: Vec<AccountId>, // Access control only
    cache: Option<ComputedValue>,   // Temporary cache OK
}
```

### Rule 5: All Storage Mutations are Messages

Storage state can only be modified through `#[ink(message)]` functions.

```rust
impl StorageCore {
    // ❌ WRONG - Not a message
    pub fn update_internal(&mut self, value: Balance) {
        self.total = value;
    }
    
    // ✅ CORRECT - Message function
    #[ink(message)]
    pub fn update_total(&mut self, value: Balance) -> Result<(), Error> {
        let caller = self.env().caller();
        self.ensure_authorized(caller)?;
        self.total = value;
        Ok(())
    }
}
```

## Implementation Guide

### 1. Structuring Your Project

```
my-protocol/
├── router/
│   ├── Cargo.toml
│   └── lib.rs          # Main entry point
├── storage-core/
│   ├── Cargo.toml
│   └── lib.rs          # Core state storage
├── logic/
│   ├── pool-logic/
│   │   ├── Cargo.toml
│   │   └── lib.rs      # Pool operations
│   └── merchant-logic/
│       ├── Cargo.toml
│       └── lib.rs      # Merchant operations
├── libraries/
│   ├── prism/
│   │   └── lib.rs      # Prism patterns
│   └── safety/
│       └── lib.rs      # Safety features
└── extensions/
    ├── price-storage/
    └── voting-storage/
```

### 2. Context Creation and Validation

```rust
// In Router
impl Router {
    fn create_context(&mut self) -> CallContext {
        let nonce = self.router_state.next_nonce();
        CallContext {
            origin: self.env().caller(),
            router: self.env().account_id(),
            timestamp: self.env().block_timestamp(),
            nonce,
            path: vec![self.env().account_id()],
        }
    }
}

// In Logic
impl LogicContract {
    fn validate_context(&self, ctx: &CallContext) -> Result<(), Error> {
        // Check router is authorized
        if !self.authorized_routers.contains(&ctx.router) {
            return Err(Error::UnauthorizedRouter);
        }
        
        // Check context age (prevent replay of old contexts)
        let current_time = self.env().block_timestamp();
        if current_time > ctx.timestamp + MAX_CONTEXT_AGE {
            return Err(Error::ContextExpired);
        }
        
        Ok(())
    }
}
```

### 3. Storage Access Pattern

```rust
// Logic contract making storage call
impl MerchantLogic {
    #[ink(message)]
    pub fn update_merchant_volume(&mut self, context: CallContext, amount: Balance) -> Result<(), Error> {
        // Validate context first
        self.validate_context(&context)?;
        
        // Use prism_call macro for cross-contract call
        prism_call!(
            self.storage_core,
            "add_merchant_volume",
            Result<(), StorageError>,
            amount
        )
        .map_err(|env_err| Error::EnvironmentError(env_err))?
        .map_err(|storage_err| Error::from(storage_err))
    }
}

// Storage contract implementation
impl StorageCore {
    #[ink(message)]
    pub fn add_merchant_volume(&mut self, amount: Balance) -> Result<(), StorageError> {
        // Get caller from environment
        let caller = self.env().caller();
        
        // Ensure caller is authorized logic
        if !self.authorized_logic.contains(&caller) {
            return Err(StorageError::UnauthorizedAccess);
        }
        
        self.merchant_volume = self.merchant_volume.saturating_add(amount);
        Ok(())
    }
}
```

## Cross-Contract Communication

### The prism_call Macro

ink! requires explicit cross-contract calls using `build_call`. The `prism_call`
macro simplifies this:

```rust
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
```

### Usage Examples

```rust
// No arguments
let balance = prism_call!(
    self.storage_core, 
    "get_total_volume", 
    Result<Balance, StorageError>
)?
.map_err(|e| Error::EnvError(e))?
.map_err(|e| Error::StorageError(e))?;

// Single argument
prism_call!(
    self.storage_core,
    "add_merchant_volume",
    Result<(), StorageError>,
    amount
)?
.map_err(|e| Error::EnvError(e))?
.map_err(|e| Error::StorageError(e))?;

// Multiple arguments
prism_call!(
    self.storage_core,
    "update_balance_with_metadata",
    Result<(), StorageError>,
    user,
    new_balance,
    timestamp,
    reason
)?
.map_err(|e| Error::EnvError(e))?
.map_err(|e| Error::StorageError(e))?;
```

### Error Handling Pattern

```rust
/// Extension trait for cleaner error handling
pub trait PrismResult<T, E> {
    fn flatten_prism_err(self) -> Result<T, Error>;
}

impl<T, E> PrismResult<T, E> for Result<Result<T, E>, ink::env::Error>
where
    E: Into<Error>,
{
    fn flatten_prism_err(self) -> Result<T, Error> {
        match self {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(contract_err)) => Err(contract_err.into()),
            Err(env_err) => Err(Error::EnvironmentError(env_err)),
        }
    }
}

// Usage
let balance = prism_call!(
    self.storage_core,
    "get_balance",
    Result<Balance, StorageError>,
    user
)
.flatten_prism_err()?;
```

## Security Model

### 1. Trust Boundaries

```
External (Untrusted)
    ↓
━━━━━━━━━━━━━━━━━━━━ Router Boundary (Validate Everything)
    ↓
Internal (Semi-Trusted)
    ↓
━━━━━━━━━━━━━━━━━━━━ Storage Boundary (Verify Caller via env())
    ↓
State (Protected)
```

### 2. Access Control Matrix

| Component | Can Call Router | Can Call Logic | Can Call Storage |
| --------- | --------------- | -------------- | ---------------- |
| External  | ✅ Yes          | ❌ No          | ❌ No            |
| Router    | N/A             | ✅ Yes         | ❌ No            |
| Logic     | ❌ No           | ❌ No          | ✅ Yes           |
| Storage   | ❌ No           | ❌ No          | N/A              |

### 3. Authorization Flow

```rust
// 1. External caller → Router
user_calls_router() {
    router.execute_operation() // ✅ Allowed
}

// 2. Router → Logic (with context)
router.execute_operation() {
    let context = self.create_context();
    logic.process(context, params) // ✅ Allowed with context
}

// 3. Logic → Storage (via cross-contract call)
logic.process(context) {
    self.validate_context(context)?; // Must validate
    prism_call!(storage, "update_state", ...) // ✅ Storage checks env().caller()
}

// 4. Storage authorization
storage.update_state() {
    let caller = self.env().caller(); // Gets logic contract address
    if !self.authorized_logic.contains(&caller) {
        return Err(Error::Unauthorized); // ❌ Blocks unauthorized
    }
}
```

## Best Practices

### 1. Always Use try_invoke

```rust
// ❌ WRONG - Using invoke
let result = build_call::<DefaultEnvironment>()
    .call(storage)
    .returns::<Result<(), Error>>()
    .invoke(); // Can panic!

// ✅ CORRECT - Using try_invoke
let result = build_call::<DefaultEnvironment>()
    .call(storage)
    .returns::<Result<(), Error>>()
    .try_invoke(); // Returns Result<Result<(), Error>, EnvError>
```

### 2. Batch Operations for Gas Efficiency

```rust
#[ink(message)]
pub fn batch_update(&mut self, updates: Vec<(AccountId, Balance)>) -> Result<Vec<Result<(), Error>>, Error> {
    self.ensure_not_paused()?;
    let context = self.create_context(); // One context for entire batch
    
    // Forward to logic with single context
    prism_call!(
        self.logic,
        "batch_process",
        Result<Vec<Result<(), Error>>, LogicError>,
        context,
        updates
    )
    .flatten_prism_err()
}
```

### 3. Event Emission at Router Level

```rust
impl Router {
    #[ink(message)]
    pub fn execute(&mut self, operation: Operation) -> Result<Response, Error> {
        let context = self.create_context();
        
        // Execute operation
        let result = self.route_operation(context.clone(), operation.clone())?;
        
        // Emit event with context for traceability
        self.env().emit_event(OperationExecuted {
            context_id: context.nonce,
            origin: context.origin,
            operation,
            result: result.clone(),
            timestamp: self.env().block_timestamp(),
        });
        
        Ok(result)
    }
}
```

### 4. Testing Strategy

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[ink::test]
    fn test_authorization_flow() {
        // Setup
        let mut storage = StorageCore::new();
        let logic = AccountId::from([0x01; 32]);
        
        // Test unauthorized access
        ink::env::test::set_caller::<DefaultEnvironment>(AccountId::from([0x99; 32]));
        assert_eq!(
            storage.update_state(100),
            Err(StorageError::UnauthorizedAccess)
        );
        
        // Authorize logic
        storage.authorize_logic(logic).unwrap();
        
        // Test authorized access
        ink::env::test::set_caller::<DefaultEnvironment>(logic);
        assert_eq!(storage.update_state(100), Ok(()));
    }
}
```

## Code Templates

### Complete Router Template

```rust
#[ink::contract]
mod router {
    use prism::{CallContext, RouterState, PrismRouter};
    use safety::{AdminControl, Pausable, ReentrancyGuard};
    
    #[ink(storage)]
    pub struct Router {
        // Safety features
        admin: AdminControl,
        pausable: Pausable,
        reentrancy_guard: ReentrancyGuard,
        
        // Prism pattern
        router_state: RouterState,
        
        // Logic contracts
        pool_logic: AccountId,
        merchant_logic: AccountId,
        
        // Storage reference
        storage_core: AccountId,
    }
    
    impl Router {
        #[ink(constructor)]
        pub fn new(storage_core: AccountId) -> Self {
            Self {
                admin: AdminControl::new(Self::env().caller()),
                pausable: Pausable::new(),
                reentrancy_guard: ReentrancyGuard::new(),
                router_state: RouterState::new(),
                pool_logic: AccountId::from([0u8; 32]),
                merchant_logic: AccountId::from([0u8; 32]),
                storage_core,
            }
        }
        
        // Admin functions
        #[ink(message)]
        pub fn register_logic(&mut self, logic_type: LogicType, logic: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            
            match logic_type {
                LogicType::Pool => self.pool_logic = logic,
                LogicType::Merchant => self.merchant_logic = logic,
            }
            
            // Register in storage
            prism_call!(
                self.storage_core,
                "authorize_logic",
                Result<(), StorageError>,
                logic
            )
            .flatten_prism_err()?;
            
            Ok(())
        }
        
        // Main routing function
        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, session_index: u32) -> Result<Balance, Error> {
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let context = self.create_context();
            
            let result = prism_call!(
                self.pool_logic,
                "update_pool_and_retrieve",
                Result<Balance, LogicError>,
                context,
                session_index
            )
            .flatten_prism_err()?;
            
            self.reentrancy_guard.exit();
            Ok(result)
        }
        
        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self, merchant_id: AccountId) -> Result<(), Error> {
            self.pausable.ensure_not_paused()?;
            self.reentrancy_guard.enter()?;
            
            let context = self.create_context();
            
            prism_call!(
                self.merchant_logic,
                "process_merchant_payment",
                Result<(), LogicError>,
                context,
                merchant_id,
                self.env().transferred_value()
            )
            .flatten_prism_err()?;
            
            self.reentrancy_guard.exit();
            Ok(())
        }
        
        // Context creation
        fn create_context(&mut self) -> CallContext {
            let nonce = self.router_state.next_nonce();
            CallContext::new(
                self.env().caller(),
                self.env().account_id(),
                self.env().block_timestamp(),
                nonce,
            )
        }
    }
}
```

### Storage Template

```rust
#[ink::contract]
mod storage_core {
    use prism::{StorageAuth, PrismStorage};
    use safety::AdminControl;
    
    #[ink(storage)]
    pub struct StorageCore {
        // Access control
        admin: AdminControl,
        storage_auth: StorageAuth,
        
        // Business state
        merchant_volume: Balance,
        total_reward_pool: Balance,
        volume_at_index: Mapping<u32, Balance>,
        
        // Legacy integration
        legacy_pool: AccountId,
    }
    
    impl StorageCore {
        #[ink(constructor)]
        pub fn new(legacy_pool: AccountId) -> Self {
            Self {
                admin: AdminControl::new(Self::env().caller()),
                storage_auth: StorageAuth::new(),
                merchant_volume: 0,
                total_reward_pool: 0,
                volume_at_index: Mapping::new(),
                legacy_pool,
            }
        }
        
        // Admin functions
        #[ink(message)]
        pub fn authorize_logic(&mut self, logic: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.storage_auth.authorize(logic)?;
            Ok(())
        }
        
        // Public read functions
        #[ink(message)]
        pub fn get_total_volume(&self) -> Balance {
            // Include legacy data
            let legacy_volume = self.get_legacy_volume();
            self.merchant_volume.saturating_add(legacy_volume)
        }
        
        // Protected write functions
        #[ink(message)]
        pub fn add_merchant_volume(&mut self, amount: Balance) -> Result<(), Error> {
            // Get caller from environment
            let caller = self.env().caller();
            
            // Verify authorization
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            // Update state
            self.merchant_volume = self.merchant_volume.saturating_add(amount);
            
            Ok(())
        }
        
        #[ink(message)]
        pub fn update_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.total_reward_pool = self.total_reward_pool.saturating_add(amount);
            Ok(())
        }
        
        // Legacy integration
        fn get_legacy_volume(&self) -> Balance {
            // Would use prism_call to query legacy contract
            0 // Placeholder
        }
    }
}
```

### Logic Template

```rust
#[ink::contract]
mod pool_logic {
    use prism::{CallContext, ExtensionRegistry, PrismLogic};
    use safety::ReentrancyGuard;
    
    #[ink(storage)]
    pub struct PoolLogic {
        // Safety
        reentrancy_guard: ReentrancyGuard,
        
        // Authorization
        authorized_routers: Vec<AccountId>,
        
        // References
        storage_core: AccountId,
        node_reward_contract: AccountId,
        
        // Extensions
        extension_registry: ExtensionRegistry,
    }
    
    impl PoolLogic {
        #[ink(constructor)]
        pub fn new(storage_core: AccountId, node_reward_contract: AccountId) -> Self {
            Self {
                reentrancy_guard: ReentrancyGuard::new(),
                authorized_routers: vec![],
                storage_core,
                node_reward_contract,
                extension_registry: ExtensionRegistry::new(storage_core),
            }
        }
        
        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, context: CallContext, session_index: u32) -> Result<Balance, Error> {
            // Validate context
            self.validate_context(&context)?;
            self.verify_caller_is_node_reward(context.origin)?;
            self.reentrancy_guard.enter()?;
            
            // Get current total volume from storage
            let total_volume = prism_call!(
                self.storage_core,
                "get_total_volume",
                Result<Balance, StorageError>
            )
            .flatten_prism_err()?;
            
            // Store session volume
            prism_call!(
                self.storage_core,
                "set_session_volume",
                Result<(), StorageError>,
                session_index,
                total_volume
            )
            .flatten_prism_err()?;
            
            // Calculate session delta
            let session_delta = self.calculate_session_delta(session_index, total_volume)?;
            
            // Calculate 3% of delta
            let three_percent = Perquintill::from_percent(3);
            let three_percent_of_delta = three_percent.mul_floor(session_delta);
            
            // Update reward pool
            prism_call!(
                self.storage_core,
                "update_reward_pool",
                Result<(), StorageError>,
                three_percent_of_delta
            )
            .flatten_prism_err()?;
            
            // Get current pool balance
            let current_pool = prism_call!(
                self.storage_core,
                "get_total_reward_pool",
                Result<Balance, StorageError>
            )
            .flatten_prism_err()?;
            
            // Calculate 10% for distribution
            let ten_percent = Perquintill::from_percent(10);
            let reward_pool = ten_percent.mul_floor(current_pool);
            
            self.reentrancy_guard.exit();
            Ok(reward_pool)
        }
        
        fn validate_context(&self, context: &CallContext) -> Result<(), Error> {
            if !self.authorized_routers.contains(&context.router) {
                return Err(Error::UnauthorizedRouter);
            }
            
            let current_time = self.env().block_timestamp();
            context.verify(current_time, 300_000)?; // 5 min max age
            
            Ok(())
        }
        
        fn verify_caller_is_node_reward(&self, caller: AccountId) -> Result<(), Error> {
            if caller != self.node_reward_contract {
                return Err(Error::OnlyCallableBy(self.node_reward_contract));
            }
            Ok(())
        }
    }
}
```

## Migration & Upgrades

### 1. Upgrading Logic Contracts

```rust
// Step 1: Deploy new logic
let new_logic = deploy_new_logic();

// Step 2: Register with router
router.register_logic(LogicType::Pool, new_logic)?;

// Step 3: Authorize in storage
storage.authorize_logic(new_logic)?;

// Step 4: Revoke old logic (after verification)
storage.revoke_logic(old_logic)?;
```

### 2. Storage Migration Pattern

```rust
impl StorageV2 {
    #[ink(message)]
    pub fn migrate_from_v1(&mut self, old_storage: AccountId) -> Result<()> {
        self.admin.ensure_admin(self.env().caller())?;
        
        // Read from old storage using prism_call
        let total_volume = prism_call!(
            old_storage,
            "get_total_volume",
            Result<Balance, Error>
        )
        .flatten_prism_err()?;
        
        // Update new storage
        self.total_volume = total_volume;
        self.migration_complete = true;
        
        Ok(())
    }
}
```

### 3. Emergency Recovery

```rust
impl Router {
    #[ink(message)]
    pub fn emergency_switch_logic(&mut self, logic_type: LogicType, backup_logic: AccountId) -> Result<()> {
        self.admin.ensure_admin(self.env().caller())?;
        self.pausable.ensure_paused()?; // Only when paused
        
        match logic_type {
            LogicType::Pool => self.pool_logic = backup_logic,
            LogicType::Merchant => self.merchant_logic = backup_logic,
        }
        
        self.env().emit_event(EmergencyLogicSwitch {
            logic_type,
            new_logic: backup_logic,
        });
        
        Ok(())
    }
}
```

## Conclusion

The Prism Doctrine provides a robust, secure, and maintainable architecture for
complex ink! smart contract systems. By following these rules and patterns, you
create contracts that can evolve with your protocol while maintaining security
and consistency.

**Remember**:

- All external calls flow through the router
- All storage access uses `#[ink(message)]` functions
- Storage validates callers via `env().caller()`
- Cross-contract calls use `prism_call!` macro with `try_invoke`
- Context flows through every call

**Key Takeaway**: _"In Prism architecture for ink!, every external call flows
through the router, every state change goes through message functions, and every
operation carries its context. No exceptions."_
