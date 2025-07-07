#![cfg_attr(not(feature = "std"), no_std)]

use scale::{Decode, Encode};

/// D9 Safety Library - Production-ready safety features for all contracts
pub mod d9_safety {
    use super::*;
    use d9_environment::D9Environment;
    use ink::primitives::AccountId;

    /// Time source abstraction

    /// Module separation for admin control
    pub mod admin {
        use super::*;

        #[derive(Debug, Clone, Encode, Decode)]
        #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
        pub struct Admin {
            current: AccountId,
            proposed: Option<AccountId>,
        }

        impl Admin {
            pub fn new(initial_admin: AccountId) -> Self {
                Self {
                    current: initial_admin,
                    proposed: None,
                }
            }

            pub fn current(&self) -> AccountId {
                self.current
            }

            /// Only current admin can propose a new admin
            pub fn propose_new(
                &mut self,
                caller: AccountId,
                new_admin: AccountId,
            ) -> Result<(), SafetyError> {
                // Critical: Verify caller is current admin
                if caller != self.current {
                    return Err(SafetyError::UnauthorizedAdmin);
                }

                // Prevent proposing zero address
                if new_admin == AccountId::from([0u8; 32]) {
                    return Err(SafetyError::InvalidAddress);
                }

                // Prevent proposing current admin (no-op)
                if new_admin == self.current {
                    return Err(SafetyError::InvalidAddress);
                }

                self.proposed = Some(new_admin);
                Ok(())
            }

            /// Proposed admin must call this to accept the role
            pub fn accept_admin(&mut self, caller: AccountId) -> Result<(), SafetyError> {
                if let Some(proposed) = self.proposed {
                    if proposed != caller {
                        return Err(SafetyError::UnauthorizedAdmin);
                    }

                    // Store old admin for event emission (if you add events)
                    let _old_admin = self.current;

                    self.current = proposed;
                    self.proposed = None;
                    Ok(())
                } else {
                    Err(SafetyError::NoProposedAdmin)
                }
            }

            /// Cancel a pending admin proposal (only current admin)
            pub fn cancel_proposal(&mut self, caller: AccountId) -> Result<(), SafetyError> {
                if caller != self.current {
                    return Err(SafetyError::UnauthorizedAdmin);
                }

                if self.proposed.is_none() {
                    return Err(SafetyError::NoProposedAdmin);
                }

                self.proposed = None;
                Ok(())
            }

            /// Get pending admin if any
            pub fn proposed(&self) -> Option<AccountId> {
                self.proposed
            }

            pub fn is_admin(&self, account: AccountId) -> bool {
                self.current == account
            }
        }
    }

    /// Pausable functionality
    pub mod pausable {
        use super::*;

        #[derive(Debug, Copy, Clone, PartialEq, Eq, Encode, Decode)]
        #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
        pub enum PauseReason {
            SecurityIncident,
            Maintenance,
            Upgrade,
            Emergency,
        }

        #[derive(Debug, Clone, Encode, Decode)]
        #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
        pub struct PausableState {
            paused: bool,
            pause_reason: Option<PauseReason>,
        }

        impl Default for PausableState {
            fn default() -> Self {
                Self {
                    paused: false,
                    pause_reason: None,
                }
            }
        }

        impl PausableState {
            pub fn pause(&mut self, reason: PauseReason) -> Result<(), SafetyError> {
                if self.paused {
                    return Err(SafetyError::AlreadyPaused);
                }
                self.paused = true;
                self.pause_reason = Some(reason);
                Ok(())
            }

            pub fn unpause(&mut self) -> Result<(), SafetyError> {
                if !self.paused {
                    return Err(SafetyError::NotPaused);
                }
                self.paused = false;
                self.pause_reason = None;
                Ok(())
            }

            pub fn ensure_not_paused(&self) -> Result<(), SafetyError> {
                if self.paused {
                    Err(SafetyError::ContractPaused)
                } else {
                    Ok(())
                }
            }

            pub fn is_paused(&self) -> bool {
                self.paused
            }
        }
    }

    pub mod reentrancy {
        use super::*;

        #[derive(Debug, Clone, Encode, Decode)]
        #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
        pub struct ReentrancyGuard {
            /// Using depth counter instead of bool for better diagnostics
            depth: u32,
            /// Track the block number of last entry for additional validation
            last_entry_block: Option<u32>,
        }

        impl Default for ReentrancyGuard {
            fn default() -> Self {
                Self {
                    depth: 0,
                    last_entry_block: None,
                }
            }
        }

        impl ReentrancyGuard {
            /// Check if currently locked without modifying state
            pub fn is_locked(&self) -> bool {
                self.depth > 0
            }

            /// Get current depth for diagnostics
            pub fn depth(&self) -> u32 {
                self.depth
            }

            /// Internal lock method - should only be called via ReentrancyScope
            fn lock(&mut self) -> Result<(), SafetyError> {
                // Check for reentrancy
                if self.depth > 0 {
                    return Err(SafetyError::ReentrantCall);
                }

                // Additional validation: check if we're in the same block
                // This helps detect cross-contract reentrancy attempts
                let current_block = ink::env::block_number::<D9Environment>();
                if let Some(last_block) = self.last_entry_block {
                    if last_block == current_block && self.depth == 0 {
                        // This might indicate state corruption
                        ink::env::debug_println!("Warning: Potential state inconsistency detected");
                    }
                }

                self.depth = 1;
                self.last_entry_block = Some(current_block);
                Ok(())
            }

            /// Internal unlock method - should only be called via ReentrancyScope
            fn unlock(&mut self) {
                if self.depth == 0 {
                    // This should never happen with correct RAII usage
                    ink::env::debug_println!(
                        "Critical: Attempting to unlock already unlocked guard"
                    );
                    return;
                }
                self.depth = 0;
                // Keep last_entry_block for diagnostics
            }

            /// Force reset - only for emergency use by admin
            pub fn force_reset(&mut self) -> Result<(), SafetyError> {
                ink::env::debug_println!("Warning: Force resetting reentrancy guard");
                self.depth = 0;
                self.last_entry_block = None;
                Ok(())
            }
        }

        /// RAII scope guard for reentrancy protection
        pub struct ReentrancyScope<'a> {
            guard: &'a mut ReentrancyGuard,
            /// Track if we successfully locked to handle drop correctly
            locked: bool,
        }

        impl<'a> ReentrancyScope<'a> {
            /// Create new scope, automatically locking the guard
            pub fn new(guard: &'a mut ReentrancyGuard) -> Result<Self, SafetyError> {
                guard.lock()?;
                Ok(Self {
                    guard,
                    locked: true,
                })
            }

            /// Manually unlock before scope ends (rare use case)
            pub fn unlock_early(mut self) -> Result<(), SafetyError> {
                if !self.locked {
                    return Err(SafetyError::InvalidState);
                }
                self.guard.unlock();
                self.locked = false;
                Ok(())
            }
        }

        impl<'a> Drop for ReentrancyScope<'a> {
            fn drop(&mut self) {
                if self.locked {
                    self.guard.unlock();
                }
            }
        }

        /// Wrapper function for safer usage pattern
        pub fn with_reentrancy_check<T, E, F>(guard: &mut ReentrancyGuard, f: F) -> Result<T, E>
        where
            F: FnOnce() -> Result<T, E>,
            E: From<SafetyError>,
        {
            let _scope = ReentrancyScope::new(guard)?;
            f()
        }
    }

    /// Error types
    #[derive(Debug, PartialEq, Eq, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum SafetyError {
        UnauthorizedAdmin,
        NoProposedAdmin,
        AlreadyPaused,
        NotPaused,
        ContractPaused,
        ReentrantCall,
        RateLimitExceeded,
        AccountFlagged,
        InvalidAddress,
        InvalidState,
        InvalidAmount,
        InvalidTimelock,
        WithdrawalNotReady,
        WithdrawalNotFound,
        UpgradeNotScheduled,
        UpgradeTooEarly,
        CircuitBreakerTripped,
        ThresholdExceeded,
        EmergencyStopActive,
    }

    /// Main safety controller
    #[derive(Debug, Encode, Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct SafetyController {
        admin: admin::Admin,
        pausable: pausable::PausableState,
        reentrancy: reentrancy::ReentrancyGuard,
    }

    impl Default for SafetyController {
        fn default() -> Self {
            Self::new(AccountId::from([0u8; 32]))
        }
    }

    impl SafetyController {
        pub fn new(initial_admin: AccountId) -> Self {
            Self {
                admin: admin::Admin::new(initial_admin),
                pausable: pausable::PausableState::default(),
                reentrancy: reentrancy::ReentrancyGuard::default(),
            }
        }

        pub fn admin(&self) -> &admin::Admin {
            &self.admin
        }

        pub fn admin_mut(&mut self) -> &mut admin::Admin {
            &mut self.admin
        }

        pub fn pausable(&self) -> &pausable::PausableState {
            &self.pausable
        }

        pub fn pausable_mut(&mut self) -> &mut pausable::PausableState {
            &mut self.pausable
        }

        pub fn reentrancy_guard(&self) -> &reentrancy::ReentrancyGuard {
            &self.reentrancy
        }

        pub fn reentrancy_guard_mut(&mut self) -> &mut reentrancy::ReentrancyGuard {
            &mut self.reentrancy
        }
    }
}

// Re-export main types
pub use d9_safety::*;
