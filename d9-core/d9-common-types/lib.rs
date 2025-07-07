#![cfg_attr(not(feature = "std"), no_std, no_main)]

use scale::{Decode, Encode};

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
    ErrorAddingVotingInterests,
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
            9 => Err(Self::ErrorAddingVotingInterests),
            _ => panic!("encountered unknown status code"),
        }
    }
}