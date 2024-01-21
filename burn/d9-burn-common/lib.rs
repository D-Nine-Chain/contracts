#![cfg_attr(not(feature = "std"), no_std, no_main)]
use ink::env::Error as EnvError;
use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
type AccountId = <D9Environment as ink::env::Environment>::AccountId;
type Balance = <D9Environment as ink::env::Environment>::Balance;
type Timestamp = <D9Environment as ink::env::Environment>::Timestamp;

/// all values are in aggregate with respect to all contracts
#[derive(scale::Decode, scale::Encode, Clone)]
#[cfg_attr(
    feature = "std",
    derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
)]
pub struct BurnPortfolio {
    /// Total amount burned through the portfolio.
    pub amount_burned: Balance,
    /// Outstanding rewards or dividends due to the portfolio.
    pub balance_due: Balance,
    /// Total rewards or dividends paid out from the portfolio.
    pub balance_paid: Balance,
    /// Timestamp or record of the last withdrawal action from the portfolio.
    pub last_withdrawal: Option<ActionRecord>,
    /// Timestamp or record of the last burn action within the portfolio.
    pub last_burn: ActionRecord,
}
impl BurnPortfolio {
    pub fn credit_burn(&mut self, amount: Balance, timestamp: Timestamp, contract: AccountId) {
        self.amount_burned = self.amount_burned.saturating_add(amount);
        self.balance_due = self.balance_due.saturating_add(amount);
        self.last_burn = ActionRecord {
            time: timestamp,
            contract: contract,
        };
    }
    pub fn update_balance(&mut self, amount: Balance, timestamp: Timestamp, contract: AccountId) {
        self.balance_due = self.balance_due.saturating_sub(amount);
        self.balance_paid = self.balance_paid.saturating_add(amount);
        self.last_withdrawal = Some(ActionRecord {
            time: timestamp,
            contract: contract,
        });
    }
}
///data structure to record the last action that was taken by an account
/// e.g. last witdrawal, last burn
#[derive(scale::Decode, scale::Encode, Clone)]
#[cfg_attr(
    feature = "std",
    derive(Debug, PartialEq, Eq, ink::storage::traits::StorageLayout, scale_info::TypeInfo)
)]
pub struct ActionRecord {
    /// timestamp of the last action in milliseconds
    pub time: Timestamp,
    /// account_id of contract that was interacted with
    pub contract: AccountId,
}

#[derive(scale::Decode, scale::Encode, Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
pub struct Account {
    ///timestamp when account created
    pub creation_timestamp: Timestamp,
    /// The total amount of assets the account has burned over time.
    pub amount_burned: Balance,
    /// The outstanding amount owed or due to the account
    pub balance_due: Balance,
    /// The total amount that has been paid out or settled to the account.
    pub balance_paid: Balance,
    /// The timestamp of the last withdrawal operation made by the account.
    pub last_withdrawal: Option<Timestamp>,
    /// The timestamp of the last burn operation conducted by the account.
    pub last_burn: Timestamp,
    /// coefficients for 0.1 and 0.01 for withdrawal calculations
    pub referral_boost_coefficients: (Balance, Balance),
    /// burn or withdrawal resets the calculation. this is teh lasts burn/withdrawal
    pub last_interaction: Timestamp,
}

impl Account {
    pub fn new(creation_timestamp: Timestamp) -> Self {
        Self {
            creation_timestamp,
            amount_burned: 0,
            balance_due: 0,
            balance_paid: 0,
            last_withdrawal: None,
            last_burn: creation_timestamp,
            last_interaction: creation_timestamp,
            referral_boost_coefficients: (0, 0),
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum Error {
    /// The burn amount provided is zero or insufficient.
    BurnAmountInsufficient,
    /// The account in question was not found.
    NoAccountFound,
    /// An attempt was made to withdraw funds within a 24-hour limit.
    EarlyWithdrawalAttempt,
    /// The contract's balance is too low to proceed.
    ContractBalanceTooLow,
    /// An invalid or unauthorized action was attempted.
    RestrictedFunction,
    /// An attempt was made to use the portfolio execute function incorrectly.
    UsePortfolioExecuteFunction,
    // a requested amount is more than what is avaiable in the balance due to the portfolio
    WithdrawalExceedsBalance,
    /// error when transfering funds
    TransferFailed,
    /// restricted function called by an unauthorized account
    InvalidCaller,
    /// The specified burn logic is not valid.
    InvalidBurnContract,
    /// main contract already has this burn contract
    BurnContractAlreadyAdded,
    /// call between contracts failed
    CrossContractCallFailed,
    /// withdrawal not permitted due to time constraint
    WithdrawalNotAllowed,
    WithdrawalAmountZero,
    ///error getting ancestors from runtime
    /// then runtime returned an empty Ancestors array. shouldnt happen but just in case
    RuntimeErrorGettingAncestors,
    NoAncestorsFound,
    MustBeMultipleOf100,
    RemoteCallToBurnContractFailed,
    RemoteCallToMiningPoolFailed,
    SomeEnvironmentError,
    CalledContractTrapped,
    CalledContractReverted,
    NotCallable,
    SomeDecodeError,
    SomeOffChainError,
    CalleeTrapped,
    CalleeReverted,
    KeyNotFound,
    _BelowSubsistenceThreshold,
    EnvironmentalTransferFailed,
    _EndowmentTooLow,
    CodeNotFound,
    Unknown,
    LoggingDisabled,
    CallRuntimeFailed,
    EcdsaRecoveryFailed,
    WithdrawalAmountExceedsBalance,
}

impl From<EnvError> for Error {
    fn from(error: EnvError) -> Self {
        match error {
            EnvError::CalleeTrapped => Self::CalledContractTrapped,
            EnvError::CalleeReverted => Self::CalledContractReverted,
            EnvError::NotCallable => Self::NotCallable,
            EnvError::KeyNotFound => Self::KeyNotFound,
            EnvError::_BelowSubsistenceThreshold => Self::_BelowSubsistenceThreshold,
            EnvError::TransferFailed => Self::EnvironmentalTransferFailed,
            EnvError::_EndowmentTooLow => Self::_EndowmentTooLow,
            EnvError::CodeNotFound => Self::CodeNotFound,
            EnvError::Unknown => Self::Unknown,
            EnvError::LoggingDisabled => Self::LoggingDisabled,
            EnvError::CallRuntimeFailed => Self::CallRuntimeFailed,
            EnvError::EcdsaRecoveryFailed => Self::EcdsaRecoveryFailed,
            _ => Self::SomeEnvironmentError,
        }
    }
}
