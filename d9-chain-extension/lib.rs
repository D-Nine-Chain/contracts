#![cfg_attr(not(feature = "std"), no_std, no_main)]
use ink::{ env::Environment, prelude::vec::Vec };

#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum D9Environment {}

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
    ) -> Result<Vec<<D9Environment as Environment>::AccountId>, RuntimeError>;

    #[ink(extension = 2)]
    fn burn(burn_amount: <D9Environment as Environment>::Balance) -> Result<(), RuntimeError>;
}

#[derive(scale::Encode, scale::Decode)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum RuntimeError {
    /// Indicates that no referral account record was found.
    ///
    /// This error is returned when an operation expects a referral account
    /// to exist, but no such record is present in the system. This could occur
    /// due to a missing or incorrect account identifier, or if the referral account
    /// was never registered.
    NoReferralAccountRecord,
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
            _ => panic!("encountered unknown status code"),
        }
    }
}
