#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract(env = D9Environment)]
mod d9_main {
    use ink::storage::Mapping;
    use d9_burn_common::{ D9Environment, BurnPortfolio, Error, ActionRecord };
    use ink::env::call::{ build_call, ExecutionInput, Selector };

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9Main {
        admin: AccountId,
        burn_contracts: Vec<AccountId>,
        /// mapping of accountId and code_hash of logic contract to respective account data
        portfolios: Mapping<AccountId, BurnPortfolio>,
    }

    impl D9Main {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, burn_contracts: Vec<AccountId>) -> Self {
            Self {
                admin,
                burn_contracts,
                portfolios: Default::default(),
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
            if !self.burn_contracts.contains(burn_contract) {
                return Err(Error::InvalidBurnContract);
            }
            let account_id: AccountId = self.env().caller();
            //todo handle contract balances too low
            // todo handle withdrawal event
        }

        #[ink(message)]
        pub fn request_payout(
            &mut self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let caller: AccountId = self.env().caller();
            //todo cases to consider. contract doesnt have enough money. the balance is too low
            if !self.contracts.contains(&caller) {
                return Err(Error::RestrictedFunction);
            }
            let maybe_portfolio = self.portfolios.get(&account_id);
            if let Some(mut portfolio) = maybe_portfolio {
                portfolio.balance_paid = portfolio.balance_paid.saturating_add(amount);

                self.portfolios.insert(account_id, &portfolio);
                Ok(());
            } else {
                Err(Error::NoAccountFound);
            }
            Ok(())
        }
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
