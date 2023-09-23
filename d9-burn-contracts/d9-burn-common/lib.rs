use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
type AccountId = <D9Environment as ink::env::Environment>::AccountId;
type Balance = <D9Environment as ink::env::Environment>::Balance;
type Timestamp = <D9Environment as ink::env::Environment>::Timestamp;

#[ink(event)]
pub struct WithdrawalExecuted {
    /// initiator of of the burn
    #[ink(topic)]
    from: AccountId,
    ///amount of tokens burned
    #[ink(topic)]
    amount: Balance,
}

#[ink(event)]
pub struct BurnExecuted {
    /// initiator of of the burn
    #[ink(topic)]
    from: AccountId,
    ///amount of tokens burned
    #[ink(topic)]
    amount: Balance,
}
#[derive(Decode, Encode)]
#[cfg_attr(
    feature = "std",
    derive(Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
/// all values are in aggregate with respect to all contracts
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
}

#[ink::trait_definition]
pub trait BurnContractInterface {
    #[ink(constructor)]
    pub fn new(master_portfolio_contract: AccountId) -> Self;

    #[ink(message)]
    pub fn total_burned(&self) -> Balance;

    #[ink(message)]
    pub fn get_account(&self, account_id: AccountId) -> Option<Account>;

    /// Executes a burn on behalf of a specified account and calculates the balance due.
    ///
    /// This function is intended to be called by the master portfolio contract to perform a burn
    /// operation for a given `account_id` and `burn_amount`. After executing the burn, it calculates
    /// the balance due for the account by subtracting any previously paid balance from the total balance
    /// due to the account.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the account for which the burn operation is to be performed.
    /// - `burn_amount`: The amount of balance the account wants to burn.
    ///
    /// # Requirements
    ///
    /// - Only the master portfolio contract can call this function. If any other caller tries to
    ///   invoke this function, it will return a `RestrictedFunction` error.
    ///
    /// # Returns
    ///
    /// - `Result<Balance, Error>`: On successful execution, the function returns the balance due after
    ///   the burn operation. If any step fails, an `Error` is returned detailing the reason for the
    ///   failure.
    ///
    /// # Errors
    ///
    /// This function can return errors in the following scenarios:
    /// - If the caller is not the master portfolio contract.
    /// - If the burn operation (`_burn`) fails.
    ///
    /// # Notes
    ///
    /// The actual burn operation is abstracted into a private function (`_burn`) for modularity
    /// and cleaner code structure.
    #[ink(message, payable)]
    pub fn portfolio_execute(
        &mut self,
        account_id: AccountId,
        burn_amount: Balance
    ) -> Result<Balance, Error>;

    /// Requests a payout for a given account from the master portfolio contract.
    ///
    /// This function initiates a call to the master portfolio contract, specifically invoking its
    /// `request_payout` method, to process a payout for the specified account and amount.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the account requesting the payout.
    /// - `amount`: The amount of balance the account is requesting as a payout.
    ///
    /// # Returns
    ///
    /// - `Result<Account, Error>`: On a successful call, returns the updated `Account` information.
    ///   On a failure, returns an `Error` detailing the reason for the failure.
    ///
    /// # Notes
    ///
    /// The actual invocation of the master portfolio's `request_payout` function is done through
    /// the `build_call` mechanism, which sets up the call environment, execution input, and expected
    /// return type. The gas limit is set to 0 for this call.
    #[ink(message)]
    pub fn request_payout(&mut self, account_id: AccountId, amount: Balance) -> Result<(), Error>;

    /// Allows a user to execute a burn and update their portfolio.
    ///
    /// This function is designed to be called by users to execute a burn and subsequently update their
    /// portfolio in the master portfolio contract. When a user calls this function, the transferred value
    /// is sent to the master portfolio contract. Afterward, the burn operation is executed and the portfolio
    /// is updated.
    ///
    /// # Requirements
    ///
    /// - The function is payable, so a balance (referred to as `amount`) must be sent along with the call.
    /// - The caller must not be the master portfolio contract. If the master portfolio contract tries to call
    ///   this function, an error (`UsePortfolioExecuteFunction`) will be returned.
    ///
    /// # Returns
    ///
    /// - `Result<Account, Error>`: On a successful execution, the function returns the updated `Account`
    ///   information. If any step fails, it returns an `Error` detailing the reason for the failure.
    ///
    /// # Errors
    ///
    /// This function can return errors in the following scenarios:
    /// - If the caller is the master portfolio contract.
    /// - If the transfer to the master portfolio contract fails.
    /// - If the burn operation (`_burn`) or the portfolio update operation (`_update_portfolio`) fails.
    ///
    /// # Notes
    ///
    /// The actual transfer to the master portfolio contract and the operations of burn and portfolio
    /// update are abstracted into private functions (`_burn` and `_update_portfolio`) for modularity
    /// and cleaner code structure.
    #[ink(message)]
    pub fn user_execute(&mut self) -> Result<Account, Error>;

    /// Processes a withdrawal request on behalf of a specified account.
    ///
    /// This function is designed to be invoked by the master portfolio contract to handle a withdrawal
    /// request for a given `account_id` and `amount`. It first attempts to process the withdrawal
    /// and then updates the portfolio to reflect this withdrawal.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the account that is requesting the withdrawal.
    /// - `amount`: The amount of balance the account is requesting to withdraw.
    ///
    /// # Requirements
    ///
    /// - Only the master portfolio contract is permitted to call this function. If any other entity
    ///   attempts to call this function, a `RestrictedFunction` error will be returned.
    ///
    /// # Returns
    ///
    /// - `Result<Account, Error>`: On successful execution, returns the updated `Account` information
    ///   post-withdrawal. If any step in the process fails, an `Error` is returned detailing the
    ///   reason for the failure.
    ///
    /// # Errors
    ///
    /// This function may return errors in the following scenarios:
    /// - If the caller is not the master portfolio contract.
    /// - If the withdrawal process (`_process_withdrawal`) or the portfolio update (`_update_portfolio`)
    ///   fails.
    ///
    /// # Notes
    ///
    /// The core operations of processing the withdrawal and updating the portfolio are abstracted into
    /// private functions (`_process_withdrawal` and `_update_portfolio`) to maintain modularity and
    /// a clean code structure.
    #[ink(message)]
    pub fn process_withdrawal(
        &self,
        account_id: AccountId,
        amount: Balance
    ) -> Result<Account, Error>;
}
