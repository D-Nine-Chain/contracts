use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;

#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
/// all values are in aggregate with respect to all contracts
pub struct BurnPortfolio {
    /// Total amount burned through the portfolio.
    amount_burned: Balance,
    /// Outstanding rewards or dividends due to the portfolio.
    balance_due: Balance,
    /// Total rewards or dividends paid out from the portfolio.
    balance_paid: Balance,
    /// Timestamp or record of the last withdrawal action from the portfolio.
    last_withdrawal: LastAction,
    /// Timestamp or record of the last burn action within the portfolio.
    last_burn: LastAction,
}

///data structure to record the last action that was taken by an account
/// e.g. last witdrawal, last burn
pub struct LastAction {
    time: Timestamp,
    contract: AccountId,
}

#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct Account {
    /// The total amount of assets the account has burned over time.
    pub amount_burned: <D9Environment as ink::env::Environment>::Balance,
    /// The outstanding amount owed or due to the account
    pub balance_due: <D9Environment as ink::env::Environment>::Balance,
    /// The total amount that has been paid out or settled to the account.
    pub balance_paid: <D9Environment as ink::env::Environment>::Balance,
    /// The timestamp of the last withdrawal operation made by the account.
    pub last_withdrawal: <D9Environment as ink::env::Environment>::Timestamp,
    /// The timestamp of the last burn operation conducted by the account.
    pub last_burn: <D9Environment as ink::env::Environment>::Timestamp,
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
}
