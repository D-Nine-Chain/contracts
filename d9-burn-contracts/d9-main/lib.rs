#![cfg_attr(not(feature = "std"), no_std, no_main)]
use d9_burn_common::{ BurnPortfolio, ActionRecord, D9Environment, Error };
#[ink::contract(env = D9Environment)]
mod d9_main {
    use super::*;
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };

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
    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9Main {
        admin: AccountId,
        burn_contracts: Vec<AccountId>,
        /// mapping of accountId and code_hash of logic contract to respective account data
        portfolios: Mapping<AccountId, BurnPortfolio>,
        /// total amount burned across all contracts
        total_amount_burned: Balance,
    }

    impl D9Main {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, burn_contracts: Vec<AccountId>) -> Self {
            Self {
                admin,
                burn_contracts,
                portfolios: Default::default(),
                total_amount_burned: Default::default(),
            }
        }
        /// Burns a specified amount from the caller's account and logs the transaction.
        ///
        /// This function allows an account to burn an amount, which is then recorded
        /// in their associated `BurnPortfolio`. The amount is deducted from the sender's
        /// balance and transferred to this main account.
        ///
        /// # Arguments
        ///
        /// * `burn_contract`: The account ID of the contract to which the burned amount will be sent.
        ///
        /// # Requirements
        ///
        /// * The caller must transfer a non-zero amount to this function.
        /// * The specified `burn_contract` must be one of the valid burn contracts recognized by this contract.
        ///
        /// # Returns
        ///
        /// * On success: Returns the updated `BurnPortfolio` for the caller.
        /// * On error: Returns an `Error` indicating the reason for the failure, such as insufficient burn amount or invalid burn contract.
        ///
        /// # Panics
        ///
        /// This function does not explicitly panic but relies on the behavior of the called burn contract.
        /// If the called burn contract reverts or fails, this function will propagate the error.
        ///
        /// # Notes
        ///
        /// * The function uses `ink::selector_bytes` to determine the function signature for the `burn_contract`.
        /// * Updates to the `BurnPortfolio` are persisted to storage.
        ///
        #[ink(message, payable)]
        pub fn burn(&mut self, burn_contract: AccountId) -> Result<BurnPortfolio, Error> {
            let burn_amount: Balance = self.env().transferred_value();
            let account_id: AccountId = self.env().caller();
            // burn amount must at least be greater than zero otherwise
            // we permit the called contract to determin its own minimum burn amount
            if burn_amount == 0 {
                return Err(Error::BurnAmountInsufficient);
            }
            if !self.burn_contracts.contains(&burn_contract) {
                return Err(Error::InvalidBurnContract);
            }

            // cross contract call
            let burn_result = build_call::<D9Environment>()
                .call(burn_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!("portfolio_execute")))
                        .push_arg(account_id)
                        .push_arg(burn_amount)
                )
                .returns::<Result<Balance, Error>>()
                .invoke();

            let last_burn = ActionRecord {
                time: self.env().block_timestamp(),
                contract: burn_contract,
            };
            self.total_amount_burned = self.total_amount_burned.saturating_add(burn_amount);
            self.env().emit_event(BurnExecuted {
                from: self.env().caller(),
                amount: burn_amount,
            });
            //process cross contract call result
            match burn_result {
                Ok(balance_increment) => {
                    if let Some(mut portfolio) = self.portfolios.get(&account_id) {
                        portfolio.amount_burned =
                            portfolio.amount_burned.saturating_add(burn_amount);
                        portfolio.balance_due =
                            portfolio.balance_due.saturating_add(balance_increment);
                        portfolio.last_burn = last_burn;
                        self.portfolios.insert(account_id, &portfolio);
                        Ok(portfolio)
                    } else {
                        let portfolio = BurnPortfolio {
                            amount_burned: burn_amount,
                            balance_due: balance_increment,
                            balance_paid: 0,
                            last_withdrawal: None,
                            last_burn: last_burn,
                        };
                        self.portfolios.insert(account_id, &portfolio);
                        Ok(portfolio)
                    }
                }
                Err(e) => Err(e),
            }
        }

        #[ink(message)]
        pub fn withdraw(&mut self, burn_contract: AccountId) -> Result<BurnPortfolio, Error> {
            if !self.burn_contracts.contains(&burn_contract) {
                return Err(Error::InvalidBurnContract);
            }
            let account_id: AccountId = self.env().caller();
            let maybe_portfolio = self.portfolios.get(&account_id);
            if maybe_portfolio.is_none() {
                return Err(Error::NoAccountFound);
            }
            let portfolio = maybe_portfolio.unwrap();
            let allowance_result = build_call::<D9Environment>()
                .call(burn_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(ink::selector_bytes!("portfolio_executed_withdrawal"))
                    ).push_arg(account_id)
                )
                .returns::<Result<(Balance, Timestamp), Error>>()
                .invoke();

            match allowance_result {
                Ok((withdrawal_allowance, last_withdrawal_timestamp)) => {
                    self.env()
                        .transfer(account_id, withdrawal_allowance)
                        .expect("Transfer failed.");
                    self.total_amount_burned =
                        self.total_amount_burned.saturating_add(withdrawal_allowance);
                    let updated_portfolio = BurnPortfolio {
                        amount_burned: portfolio.amount_burned,
                        balance_due: portfolio.balance_due.saturating_sub(withdrawal_allowance),
                        balance_paid: portfolio.balance_paid.saturating_add(withdrawal_allowance),
                        last_withdrawal: Some(ActionRecord {
                            time: last_withdrawal_timestamp,
                            contract: burn_contract,
                        }),
                        last_burn: portfolio.last_burn,
                    };
                    self.portfolios.insert(account_id, &updated_portfolio);
                    Ok(updated_portfolio)
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        //todo add to get portfolio of account
        //todo add function to add burn contrat
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;

        /// We test if the default constructor does its job.
        #[ink::test]
        fn default_works() {
            let d9referral = D9BurnMining::default();
            assert_eq!(d9referral.get(), false);
        }

        /// We test a simple use case of our contract.
        #[ink::test]
        fn it_works() {
            let mut d9referral = D9BurnMining::new(false);
            assert_eq!(d9referral.get(), false);
            d9referral.flip();
            assert_eq!(d9referral.get(), true);
        }
    }
}
