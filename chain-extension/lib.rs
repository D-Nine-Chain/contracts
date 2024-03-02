#![cfg_attr(not(feature = "std"), no_std, no_main)]
use ink::{ env::Environment, prelude::vec::Vec, primitives::AccountId };
use scale::{ Decode, Encode };
use sp_arithmetic::Perquintill;
// use sp_staking::SessionIndex;
// use scale_info::TypeInfo;
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub struct D9Environment {}

impl Environment for D9Environment {
    const MAX_EVENT_TOPICS: usize = <ink::env::DefaultEnvironment as Environment>::MAX_EVENT_TOPICS;
    type AccountId = <ink::env::DefaultEnvironment as Environment>::AccountId;
    type Balance = <ink::env::DefaultEnvironment as Environment>::Balance;
    type Hash = <ink::env::DefaultEnvironment as Environment>::Hash;
    type BlockNumber = <ink::env::DefaultEnvironment as Environment>::BlockNumber;
    type Timestamp = <ink::env::DefaultEnvironment as Environment>::Timestamp;

    type ChainExtension = D9ChainExtension;
}

#[ink::chain_extension]
pub trait D9ChainExtension {
    type ErrorCode = RuntimeError;

    #[ink(extension = 0)]
    fn get_referree_parent(
        referree: <D9Environment as Environment>::AccountId
    ) -> Option<<D9Environment as Environment>::AccountId>;

    #[ink(extension = 1)]
    fn get_ancestors(
        referree: <D9Environment as Environment>::AccountId
    ) -> Result<Option<Vec<<D9Environment as Environment>::AccountId>>, RuntimeError>;

    #[ink(extension = 2)]
    fn get_validators() -> Result<Vec<<D9Environment as Environment>::AccountId>, RuntimeError>;

    #[ink(extension = 3)]
    fn get_session_node_list(
        session_index: u32
    ) -> Result<Vec<<D9Environment as Environment>::AccountId>, RuntimeError>;

    #[ink(extension = 4)]
    fn get_current_session_index() -> Result<u32, RuntimeError>;

    #[ink(extension = 5)]
    fn get_user_supported_nodes(user_id: AccountId) -> Result<Vec<AccountId>, RuntimeError>;

    #[ink(extension = 6)]
    fn get_node_sharing_percentage(node_id: AccountId) -> Result<u8, RuntimeError>;

    #[ink(extension = 7)]
    fn get_user_vote_ratio_for_candidate(
        user_id: AccountId,
        node_id: AccountId
    ) -> Result<Option<Perquintill>, RuntimeError>;

    #[ink(extension = 8)]
    fn get_active_validators() -> Result<Vec<AccountId>, RuntimeError>;
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum RuntimeError {
    NoReferralAccountRecord,
    ErrorGettingValidators,
    ErrorGettingSessionList,
    ErrorGettingSession,
    ErrorGettingNodesUserSupports,
    ErrorGettingNodeSharingPercentage,
    ErrorGettingUserVoteRatioForCandidate,
    ErrorGettingCurrentValidators,
}

impl From<scale::Error> for RuntimeError {
    fn from(_: scale::Error) -> Self {
        panic!("encountered unexpected invalid SCALE encoding")
    }
}

impl ink::env::chain_extension::FromStatusCode for RuntimeError {
    fn from_status_code(status_code: u32) -> Result<(), Self> {
        match status_code {
            0 => Ok(()),
            1 => Err(Self::NoReferralAccountRecord),
            2 => Err(Self::ErrorGettingValidators),
            3 => Err(Self::ErrorGettingSessionList),
            4 => Err(Self::ErrorGettingSession),
            5 => Err(Self::ErrorGettingNodesUserSupports),
            6 => Err(Self::ErrorGettingNodeSharingPercentage),
            7 => Err(Self::ErrorGettingUserVoteRatioForCandidate),
            8 => Err(Self::ErrorGettingCurrentValidators),
            _ => panic!("encountered unknown status code"),
        }
    }
}
