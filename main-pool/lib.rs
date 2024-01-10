#![cfg_attr(not(feature = "std"), no_std, no_main)]
use d9_burn_common::{ BurnPortfolio, ActionRecord, D9Environment, Error };
#[ink::contract(env = D9Environment)]
mod d9_main_pool {
    use super::*;
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9MainPool {
        admin: AccountId,
        burn_contracts: Vec<AccountId>,
        /// mapping of accountId and code_hash of logic contract to respective account data
        portfolios: Mapping<AccountId, BurnPortfolio>,
        /// total amount burned across all contracts
        total_amount_burned: Balance,
        node_reward_contract: AccountId,
    }

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

    // /pdate_balance(remainder, last_withdrawal_timestamp, burn_contract);
    impl D9MainPool {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor, payable)]
        pub fn new(
            admin: AccountId,
            burn_contracts: Vec<AccountId>,
            node_reward_contract: AccountId
        ) -> Self {
            Self {
                admin,
                burn_contracts,
                node_reward_contract,
                portfolios: Default::default(),
                total_amount_burned: Default::default(),
            }
        }

        #[ink(message, payable)]
        pub fn burn(
            &mut self,
            burn_beneficiary: AccountId,
            burn_contract: AccountId
        ) -> Result<BurnPortfolio, Error> {
            let caller = self.env().caller();
            let burn_amount = self.env().transferred_value();

            // Ensure the burn amount is sufficient
            if burn_amount == 0 {
                return Err(Error::BurnAmountInsufficient);
            }

            // Verify the burn contract
            if !self.burn_contracts.contains(&burn_contract) {
                return Err(Error::InvalidBurnContract);
            }

            // Make the cross-contract call
            let burn_result = self.call_burn_contract(burn_beneficiary, burn_amount, burn_contract);
            if burn_result.is_err() {
                return Err(Error::RemoteCallToBurnContractFailed);
            }
            let balance_increase = burn_result.unwrap();

            // Update portfolio and total burn
            let last_burn = ActionRecord {
                time: self.env().block_timestamp(),
                contract: burn_contract,
            };
            self.total_amount_burned = self.total_amount_burned.saturating_add(burn_amount);

            let mut portfolio = self.portfolios.get(burn_beneficiary).unwrap_or(BurnPortfolio {
                amount_burned: 0,
                balance_due: 0,
                balance_paid: 0,
                last_withdrawal: None,
                last_burn: last_burn.clone(), // clone required for new portfolios
            });
            portfolio.amount_burned = portfolio.amount_burned.saturating_add(burn_amount);
            portfolio.balance_due = portfolio.balance_due.saturating_add(balance_increase);
            portfolio.last_burn = last_burn;

            // Emit an event for the burn execution
            self.env().emit_event(BurnExecuted {
                from: caller,
                amount: burn_amount,
            });
            self.portfolios.insert(burn_beneficiary, &portfolio);
            Ok(portfolio.clone()) // clone for returning; original is in the map
        }

        #[ink(message)]
        pub fn withdraw(&mut self, burn_contract: AccountId) -> Result<BurnPortfolio, Error> {
            // Check if the contract is valid
            if !self.burn_contracts.contains(&burn_contract) {
                return Err(Error::InvalidBurnContract);
            }

            let account_id: AccountId = self.env().caller();
            let mut portfolio = self.portfolios.get(&account_id).ok_or(Error::NoAccountFound)?;

            // Get the withdrawal allowance and timestamp
            let (withdraw_allowance, this_withdrawal_timestamp) = self.get_withdrawal_allowance(
                burn_contract,
                account_id
            )?;

            // If there's no allowance, return early
            if withdraw_allowance == 0 {
                return Ok(portfolio);
            }

            // If no ancestors are found or payment fails, process withdrawal normally
            portfolio.update_balance(withdraw_allowance, this_withdrawal_timestamp, burn_contract);
            self.portfolios.insert(account_id, &portfolio);

            self
                .env()
                .transfer(account_id, withdraw_allowance)
                .map_err(|_| Error::TransferFailed)?;
            Ok(portfolio.clone())
        }

        #[ink(message)]
        pub fn get_ancestors(&self, account_id: AccountId) -> Option<Vec<AccountId>> {
            let result = self.env().extension().get_ancestors(account_id);
            match result {
                Ok(ancestors) => ancestors,
                Err(_) => None,
            }
        }

        #[ink(message)]
        pub fn add_burn_contract(&mut self, burn_contract: AccountId) -> Result<(), Error> {
            if self.burn_contracts.contains(&burn_contract) {
                return Err(Error::BurnContractAlreadyAdded);
            }
            if self.env().caller() != self.admin {
                return Err(Error::InvalidCaller);
            }
            self.burn_contracts.push(burn_contract);

            Ok(())
        }

        #[ink(message)]
        pub fn remove_burn_contract(&mut self, burn_contract: AccountId) {
            assert!(self.burn_contracts.contains(&burn_contract), "BurnContract not found");
            // assert!(self.env().caller() != self.admin, "Invalid caller");
            self.burn_contracts.retain(|&x| x != burn_contract);
        }
        #[ink(message)]
        pub fn get_admin(&self) -> AccountId {
            self.admin
        }

        #[ink(message)]
        pub fn get_total_burned(&self) -> Balance {
            self.total_amount_burned
        }

        #[ink(message)]
        pub fn get_portfolio(&self, account_id: AccountId) -> Option<BurnPortfolio> {
            self.portfolios.get(&account_id)
        }

        #[ink(message)]
        pub fn get_balance(&self) -> Balance {
            self.env().balance()
        }

        fn callable_by(&self, account_id: AccountId) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != account_id {
                return Err(Error::InvalidCaller);
            }
            Ok(())
        }

        #[ink(message)]
        pub fn pay_node_reward(
            &mut self,
            node_id: AccountId,
            node_reward: Balance
        ) -> Result<(), Error> {
            let check = self.callable_by(self.node_reward_contract);
            assert!(check.is_ok(), "Invalid caller");
            let result = self.env().transfer(node_id, node_reward);
            match result {
                Ok(_) => Ok(()),
                Err(_) => Err(Error::TransferFailed),
            }
        }

        #[ink(message)]
        pub fn set_node_reward_contract(&mut self, node_reward_contract: AccountId) {
            let check = self.callable_by(self.admin);
            assert!(check.is_ok(), "Invalid caller");
            self.node_reward_contract = node_reward_contract;
        }

        fn call_burn_contract(
            &self,
            account_id: AccountId,
            burn_amount: Balance,
            burn_contract: AccountId
        ) -> Result<Balance, Error> {
            build_call::<D9Environment>()
                .call(burn_contract)
                .gas_limit(0) // replace with an appropriate gas limit
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!("initiate_burn")))
                        .push_arg(account_id)
                        .push_arg(burn_amount)
                )
                .returns::<Result<Balance, Error>>()
                .invoke()
        }

        fn get_withdrawal_allowance(
            &self,
            burn_contract: AccountId,
            account_id: AccountId
        ) -> Result<(Balance, Timestamp), Error> {
            build_call::<D9Environment>()
                .call(burn_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(ink::selector_bytes!("prepare_withdrawal"))
                    ).push_arg(account_id)
                )
                .returns::<Result<(Balance, Timestamp), Error>>()
                .invoke()
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use d9_main_pool::*;
    }
    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        use super::*;
        use ink_e2e::build_message;
        use d9_burn_mining::d9_burn_mining::D9burnMining;
        use d9_burn_mining::d9_burn_mining::D9burnMiningRef;
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        #[ink_e2e::test]
        async fn burn_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            //prepare main contract
            let burn_manager_constructor = D9BurnManagerRef::new(
                ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                vec![]
            );
            let main_contract_address = client
                .instantiate(
                    "d9_burn_manager",
                    &ink_e2e::alice(),
                    burn_manager_constructor,
                    0,
                    None
                ).await
                .expect("Failed to instantiate contract").account_id;

            //prepare burn contract
            let burn_constructor = D9burnMiningRef::new(main_contract_address, 100);
            let burn_contract_address = client
                .instantiate("d9_burn_mining", &ink_e2e::alice(), burn_constructor, 0, None).await
                .expect("Failed to instantiate contract").account_id;

            // add burn contract to main contract
            let add_burn_contract_call = build_message::<D9BurnManagerRef>(
                main_contract_address.clone()
            ).call(|d9_burn_manager| d9_main_pool.add_burn_contract(burn_contract_address.clone()));

            let add_burn_contract_response = client.call(
                &ink_e2e::alice(),
                add_burn_contract_call,
                0,
                None
            ).await;

            assert!(add_burn_contract_response.is_ok());

            let burn_call = build_message::<D9BurnManagerRef>(main_contract_address.clone()).call(
                |d9_burn_manager| d9_main_pool.burn(burn_contract_address.clone())
            );
            let burn_amount = 500;
            let burn_response = client.call(
                &ink_e2e::alice(),
                burn_call,
                burn_amount.clone(),
                None
            ).await;

            assert!(burn_response.is_ok());

            let get_burn_amount_call = build_message::<D9BurnManagerRef>(
                main_contract_address.clone()
            ).call(|d9_burn_manager| d9_main_pool.get_total_burned());

            let get_burn_amount_response = client.call(
                &ink_e2e::alice(),
                get_burn_amount_call,
                0,
                None
            ).await;

            assert!(get_burn_amount_response.is_ok());
            let total_burned = get_burn_amount_response.unwrap().return_value();
            assert_eq!(total_burned, burn_amount);
            Ok(())
        }

        #[ink_e2e::test]
        async fn withdraw_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            //prepare main contract
            let burn_manager_constructor = D9BurnManagerRef::new(
                ink_e2e::account_id(ink_e2e::AccountKeyring::Alice),
                vec![]
            );
            let main_contract_address = client
                .instantiate(
                    "d9_burn_manager",
                    &ink_e2e::alice(),
                    burn_manager_constructor,
                    1000000000000,
                    None
                ).await
                .expect("Failed to instantiate main contract").account_id;

            //prepare burn contract
            let burn_constructor = D9burnMiningRef::new(main_contract_address, 100);
            let burn_contract_address = client
                .instantiate("d9_burn_mining", &ink_e2e::alice(), burn_constructor, 0, None).await
                .expect("Failed to instantiate burn contract").account_id;

            // add burn contract to main contract
            let add_burn_contract_call = build_message::<D9BurnManagerRef>(
                main_contract_address.clone()
            ).call(|d9_burn_manager| d9_main_pool.add_burn_contract(burn_contract_address.clone()));

            let add_burn_contract_response = client.call(
                &ink_e2e::alice(),
                add_burn_contract_call,
                0,
                None
            ).await;

            assert!(add_burn_contract_response.is_ok());

            let burn_call = build_message::<D9BurnManagerRef>(main_contract_address.clone()).call(
                |d9_burn_manager| d9_main_pool.burn(burn_contract_address.clone())
            );
            let burn_amount = 500;
            let burn_response = client.call(
                &ink_e2e::alice(),
                burn_call,
                burn_amount.clone(),
                None
            ).await;

            assert!(burn_response.is_ok());

            let withdraw_call = build_message::<D9BurnManagerRef>(
                main_contract_address.clone()
            ).call(|d9_burn_manager| d9_main_pool.withdraw(burn_contract_address.clone()));
            let withdraw_response = client.call(&ink_e2e::alice(), withdraw_call, 0, None).await;
            assert!(withdraw_response.is_ok());
            Ok(())
        }
    }
}
