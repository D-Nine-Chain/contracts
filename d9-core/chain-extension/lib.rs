#![cfg_attr(not(feature = "std"), no_std, no_main)]
#![allow(clippy::type_complexity)]
use ink::{env::Environment, prelude::vec::Vec, primitives::AccountId};
use sp_arithmetic::Perquintill;
use d9_common_types::RuntimeError;

pub use d9_environment::D9Environment;
pub struct D9EnvironmentWithChainExtension;

impl Environment for D9EnvironmentWithChainExtension {
    const MAX_EVENT_TOPICS: usize = <D9Environment as Environment>::MAX_EVENT_TOPICS;
    type AccountId = <D9Environment as Environment>::AccountId;
    type Balance = <D9Environment as Environment>::Balance;
    type Hash = <D9Environment as Environment>::Hash;
    type BlockNumber = <D9Environment as Environment>::BlockNumber;
    type Timestamp = <D9Environment as Environment>::Timestamp;

    type ChainExtension = D9ChainExtension;
}

#[ink::chain_extension]
pub trait D9ChainExtension {
    type ErrorCode = RuntimeError;

    #[ink(extension = 0)]
    fn get_referree_parent(
        referree: <D9EnvironmentWithChainExtension as Environment>::AccountId,
    ) -> Option<<D9EnvironmentWithChainExtension as Environment>::AccountId>;

    #[ink(extension = 1)]
    fn get_ancestors(
        referree: <D9EnvironmentWithChainExtension as Environment>::AccountId,
    ) -> Result<Option<Vec<<D9EnvironmentWithChainExtension as Environment>::AccountId>>, RuntimeError>;

    #[ink(extension = 2)]
    fn get_validators() -> Result<Vec<<D9EnvironmentWithChainExtension as Environment>::AccountId>, RuntimeError>;

    #[ink(extension = 3)]
    fn get_session_node_list(
        session_index: u32,
    ) -> Result<Vec<<D9EnvironmentWithChainExtension as Environment>::AccountId>, RuntimeError>;

    #[ink(extension = 4)]
    fn get_current_session_index() -> Result<u32, RuntimeError>;

    #[ink(extension = 5)]
    fn get_user_supported_nodes(user_id: AccountId) -> Result<Vec<AccountId>, RuntimeError>;

    #[ink(extension = 6)]
    fn get_node_sharing_percentage(node_id: AccountId) -> Result<u8, RuntimeError>;

    #[ink(extension = 7)]
    fn get_user_vote_ratio_for_candidate(
        user_id: AccountId,
        node_id: AccountId,
    ) -> Result<Option<Perquintill>, RuntimeError>;

    #[ink(extension = 8)]
    fn get_active_validators() -> Result<Vec<AccountId>, RuntimeError>;

    #[ink(extension = 9)]
    fn add_voting_interests(
        vote_delegator: AccountId,
        voting_interests: u64,
    ) -> Result<(), RuntimeError>;
}

