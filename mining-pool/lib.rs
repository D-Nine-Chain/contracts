#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod mining_pool {
    use super::*;
    use ink::storage::Mapping;
    use scale::{ Decode, Encode };
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;
    // use substrate_fixed::{ FixedU128, types::extra::U12 };
    // type FixedBalance = FixedU128<U12>;

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Currency {
        D9,
        USDT,
    }
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Direction(Currency, Currency);

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        OnlyCallableBy(AccountId),
        FailedToGetExchangeAmount,
        FailedToTransferD9ToUser,
        SessionPoolNotReady,
    }

    #[ink(storage)]
    pub struct MiningPool {
        admin: AccountId,
        reward_pallet: AccountId,
        main_contract: AccountId,
        merchant_contract: AccountId,
        node_reward_contract: AccountId,
        amm_contract: AccountId,
        total_merchant_processed_d9: Balance,
        session_total_processed_d9: Mapping<u32, Balance>,
        last_session: u32,
    }

    impl MiningPool {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(
            main_contract: AccountId,
            merchant_contract: AccountId,
            node_reward_contract: AccountId,
            amm_contract: AccountId
        ) -> Self {
            Self {
                admin: Self::env().caller(),
                main_contract,
                node_reward_contract,
                merchant_contract,
                amm_contract,
                total_merchant_processed_d9: 0,
                session_total_processed_d9: Mapping::new(),
                reward_pallet: [0u8; 32].into(),
                last_session: 0,
            }
        }

        #[ink(message)]
        pub fn pay_node_reward(
            &mut self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let _ = self.only_callable_by(self.node_reward_contract)?;
            let _ = self.env().transfer(account_id, amount);
            Ok(())
        }

        #[ink(message)]
        pub fn update_session_total(&mut self, ending_session: u32) -> Result<(), Error> {
            let _ = self.only_callable_by(self.reward_pallet)?;
            let total_burned = self.get_total_burned();
            let new_session_total = total_burned.saturating_add(self.total_merchant_processed_d9);
            self.session_total_processed_d9.insert(ending_session, &new_session_total);
            self.last_session = ending_session;
            Ok(())
        }

        pub fn get_last_session_pool(&self) -> Result<Balance, Error> {
            self.calculate_session_pool(self.last_session)
        }

        /// session pool is defined as the difference between session_index total and previous session total
        ///
        /// we want a delta value
        #[ink(message)]
        pub fn calculate_session_pool(&self, session_index: u32) -> Result<Balance, Error> {
            let session_total_opt = self.session_total_processed_d9.get(&session_index);
            let session_total = match session_total_opt {
                Some(total) => total,
                None => {
                    return Err(Error::SessionPoolNotReady);
                }
            };
            let previous_session = session_index - 1;
            let previous_session_total = self.session_total_processed_d9
                .get(&previous_session)
                .unwrap_or(0);
            let session_pool = session_total.saturating_sub(previous_session_total);
            Ok(session_pool)
        }

        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self) -> Result<(), Error> {
            let _ = self.only_callable_by(self.merchant_contract)?;
            let received_amount = self.env().transferred_value();
            self.total_merchant_processed_d9 =
                self.total_merchant_processed_d9.saturating_add(received_amount);
            Ok(())
        }

        #[ink(message)]
        pub fn merchant_user_redeem_d9(
            &self,
            user_account: AccountId,
            redeemable_usdt: Balance
        ) -> Result<Balance, Error> {
            let _ = self.only_callable_by(self.merchant_contract)?;

            let amount_request = self.get_exchange_amount(
                Direction(Currency::USDT, Currency::D9),
                redeemable_usdt
            );
            if amount_request.is_err() {
                return Err(Error::FailedToGetExchangeAmount);
            }
            let d9_amount = amount_request.unwrap();
            let transfer_to_user_result = self.env().transfer(user_account, d9_amount);
            if transfer_to_user_result.is_err() {
                return Err(Error::FailedToTransferD9ToUser);
            }
            Ok(d9_amount)
        }

        fn get_exchange_amount(
            &self,
            direction: Direction,
            amount: Balance
        ) -> Result<Balance, Error> {
            build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("calculate_exchange")))
                        .push_arg(direction)
                        .push_arg(amount)
                )
                .returns::<Result<Balance, Error>>()
                .invoke()
        }

        fn get_total_burned(&self) -> Balance {
            build_call::<D9Environment>()
                .call(self.main_contract)
                .gas_limit(0)
                .exec_input(ExecutionInput::new(Selector::new(selector_bytes!("get_total_burned"))))
                .returns::<Balance>()
                .invoke()
        }

        #[ink(message)]
        pub fn change_merchant_contract(
            &mut self,
            merchant_contract: AccountId
        ) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            self.merchant_contract = merchant_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_reward_pallet(&mut self, reward_pallet: AccountId) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin)?;
            self.reward_pallet = reward_pallet;
            Ok(())
        }

        #[ink(message)]
        pub fn change_node_reward_contract(
            &mut self,
            node_reward_contract: AccountId
        ) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            self.node_reward_contract = node_reward_contract;
            Ok(())
        }
        #[ink(message)]
        pub fn change_amm_contract(&mut self, amm_contract: AccountId) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            self.amm_contract = amm_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_main_contract(&mut self, main_contract: AccountId) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            self.main_contract = main_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn set_code(&mut self, code_hash: [u8; 32]) {
            let caller = self.env().caller();
            assert!(caller == self.admin, "Only admin can set code hash.");
            ink::env
                ::set_code_hash(&code_hash)
                .unwrap_or_else(|err| {
                    panic!("Failed to `set_code_hash` to {:?} due to {:?}", code_hash, err)
                });
            ink::env::debug_println!("Switched code hash to {:?}.", code_hash);
        }

        fn only_callable_by(&self, account_id: AccountId) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != account_id {
                return Err(Error::OnlyCallableBy(account_id));
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

        //   #[ink::test]
        //   fn it_works() {
        //       let mut mining_pool = MiningPool::new(false);
        //       assert_eq!(mining_pool.get(), false);
        //       mining_pool.flip();
        //       assert_eq!(mining_pool.get(), true);
        //   }
    }

    /// This is how you'd write end-to-end (E2E) or integration tests for ink! contracts.
    ///
    /// When running these you need to make sure that you:
    /// - Compile the tests with the `e2e-tests` feature flag enabled (`--features e2e-tests`)
    /// - Are running a Substrate node which contains `pallet-contracts` in the background
    #[cfg(all(test, feature = "e2e-tests"))]
    mod e2e_tests {
        /// Imports all the definitions from the outer scope so we can use them here.
        use super::*;

        /// A helper function used for calling contract messages.
        use ink_e2e::build_message;

        /// The End-to-End test `Result` type.
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        /// We test that we can upload and instantiate the contract using its default constructor.
        #[ink_e2e::test]
        async fn default_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = MiningPoolRef::default();

            // When
            let contract_account_id = client
                .instantiate("mining_pool", &ink_e2e::alice(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            // Then
            let get = build_message::<MiningPoolRef>(contract_account_id.clone()).call(|mining_pool|
                mining_pool.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::alice(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            Ok(())
        }

        /// We test that we can read and write a value from the on-chain contract contract.
        #[ink_e2e::test]
        async fn it_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = MiningPoolRef::new(false);
            let contract_account_id = client
                .instantiate("mining_pool", &ink_e2e::bob(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            let get = build_message::<MiningPoolRef>(contract_account_id.clone()).call(|mining_pool|
                mining_pool.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            // When
            let flip = build_message::<MiningPoolRef>(contract_account_id.clone()).call(
                |mining_pool| mining_pool.flip()
            );
            let _flip_result = client
                .call(&ink_e2e::bob(), flip, 0, None).await
                .expect("flip failed");

            // Then
            let get = build_message::<MiningPoolRef>(contract_account_id.clone()).call(|mining_pool|
                mining_pool.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), true));

            Ok(())
        }
    }
}
