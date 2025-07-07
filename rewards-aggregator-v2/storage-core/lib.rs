#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[ink::contract]
mod rewards_aggregator_storage {
    use prism::{StorageAuth, PrismStorage, PrismError};
    use prism::prism_call;
    use safety::{AdminControl, SafetyError};
    use ink::storage::Mapping;
    
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        UnauthorizedAccess,
        SafetyError(SafetyError),
        PrismError(PrismError),
        EnvironmentError,
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
    pub struct RewardsAggregatorStorage {
        admin: AdminControl,
        storage_auth: StorageAuth,
        merchant_volume: Balance,
        accumulative_reward_pool: Balance,
        last_session: u32,
        volume_at_index: Mapping<u32, Balance>,
        highest_price: Balance,
        legacy_mining_pool: AccountId,
    }
    
    impl RewardsAggregatorStorage {
        #[ink(constructor)]
        pub fn new(legacy_mining_pool: AccountId) -> Self {
            Self {
                admin: AdminControl::new(Self::env().caller()),
                storage_auth: StorageAuth::new(),
                merchant_volume: 0,
                accumulative_reward_pool: 0,
                last_session: 0,
                volume_at_index: Mapping::new(),
                highest_price: 0,
                legacy_mining_pool,
            }
        }
        
        // Admin functions
        #[ink(message)]
        pub fn authorize_logic(&mut self, logic: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.storage_auth.authorize(logic)?;
            Ok(())
        }
        
        #[ink(message)]
        pub fn revoke_logic(&mut self, logic: AccountId) -> Result<(), Error> {
            self.admin.ensure_admin(self.env().caller())?;
            self.storage_auth.authorized_logic.retain(|&l| l != logic);
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
        
        // Public read functions (combining legacy + new data)
        #[ink(message)]
        pub fn get_total_merchant_volume(&self) -> Balance {
            let old_volume = self.get_legacy_merchant_volume();
            self.merchant_volume.saturating_add(old_volume)
        }
        
        #[ink(message)]
        pub fn get_total_reward_pool(&self) -> Balance {
            let old_pool = self.get_legacy_reward_pool();
            self.accumulative_reward_pool.saturating_add(old_pool)
        }
        
        #[ink(message)]
        pub fn get_total_volume(&self) -> Balance {
            let total_burned = self.get_legacy_total_burned();
            let total_merchant = self.get_total_merchant_volume();
            total_burned.saturating_add(total_merchant)
        }
        
        #[ink(message)]
        pub fn get_session_volume(&self, session_index: u32) -> Balance {
            self.volume_at_index.get(session_index).unwrap_or(0)
        }
        
        #[ink(message)]
        pub fn get_last_session(&self) -> u32 {
            self.last_session
        }
        
        #[ink(message)]
        pub fn get_highest_price(&self) -> Balance {
            self.highest_price
        }
        
        #[ink(message)]
        pub fn get_admin(&self) -> AccountId {
            self.admin.get_admin()
        }
        
        // Protected write functions (only authorized logic can call)
        #[ink(message)]
        pub fn add_merchant_volume(&mut self, amount: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.merchant_volume = self.merchant_volume.saturating_add(amount);
            Ok(())
        }
        
        #[ink(message)]
        pub fn update_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_add(amount);
            Ok(())
        }
        
        #[ink(message)]
        pub fn subtract_from_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_sub(amount);
            Ok(())
        }
        
        #[ink(message)]
        pub fn set_session_volume(&mut self, session_index: u32, volume: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.volume_at_index.insert(session_index, &volume);
            Ok(())
        }
        
        #[ink(message)]
        pub fn set_last_session(&mut self, session: u32) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.last_session = session;
            Ok(())
        }
        
        #[ink(message)]
        pub fn set_highest_price(&mut self, price: Balance) -> Result<(), Error> {
            let caller = self.env().caller();
            if !self.storage_auth.is_authorized(caller) {
                return Err(Error::UnauthorizedAccess);
            }
            
            self.highest_price = price;
            Ok(())
        }
        
        // Legacy data access (internal helpers)
        fn get_legacy_merchant_volume(&self) -> Balance {
            prism_call!(
                self.legacy_mining_pool,
                "get_merchant_volume",
                Balance
            ).unwrap_or_else(|_| Ok(0)).unwrap_or(0)
        }
        
        fn get_legacy_reward_pool(&self) -> Balance {
            prism_call!(
                self.legacy_mining_pool,
                "get_accumulative_reward_pool",
                Balance
            ).unwrap_or_else(|_| Ok(0)).unwrap_or(0)
        }
        
        fn get_legacy_total_burned(&self) -> Balance {
            prism_call!(
                self.legacy_mining_pool,
                "get_total_burned",
                Balance
            ).unwrap_or_else(|_| Ok(0)).unwrap_or(0)
        }
    }
    
    impl PrismStorage for RewardsAggregatorStorage {
        fn verify_token(&self, _token: &prism::StorageAccessToken) -> Result<(), PrismError> {
            // Not using token-based access, using direct caller verification
            Ok(())
        }
        
        fn authorize_logic(&mut self, logic: AccountId) -> Result<(), PrismError> {
            self.storage_auth.authorize(logic)
        }
        
        fn revoke_logic(&mut self, logic: AccountId) -> Result<(), PrismError> {
            self.storage_auth.authorized_logic.retain(|&l| l != logic);
            Ok(())
        }
        
        fn is_authorized(&self, logic: AccountId) -> bool {
            self.storage_auth.is_authorized(logic)
        }
    }
    
}