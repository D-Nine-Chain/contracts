use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
type AccountId = <D9Environment as ink::env::Environment>::AccountId;
type Balance = <D9Environment as ink::env::Environment>::Balance;
type Timestamp = <D9Environment as ink::env::Environment>::Timestamp;

/// all values are in aggregate with respect to all contracts
#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
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

///data structure to record the last action that was taken by an account
/// e.g. last witdrawal, last burn
#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct ActionRecord {
    pub time: Timestamp,
    pub contract: AccountId,
}

#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct Account {
    /// The total amount of assets the account has burned over time.
    pub amount_burned: Balance,
    /// The outstanding amount owed or due to the account
    pub balance_due: Balance,
    /// The total amount that has been paid out or settled to the account.
    pub balance_paid: Balance,
    /// The timestamp of the last withdrawal operation made by the account.
    pub last_withdrawal: Timestamp,
    /// The timestamp of the last burn operation conducted by the account.
    pub last_burn: Timestamp,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum Error {
    /// The burn amount provided is zero or insufficient.
    BurnAmountInsufficient,
    /// The specified burn logic is not valid.
    InvalidBurnContract,
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
    // error when transfering funds
    TransferFailed(AccountId, Balance),
}
