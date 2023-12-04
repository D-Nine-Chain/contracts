#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod mining_pool {
    use super::*;
    use scale::{ Decode, Encode };
    use sp_arithmetic::Perbill;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;

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
    }

    #[ink(storage)]
    pub struct MiningPool {
        main_contract: AccountId,
        merchant_contract: AccountId,
        node_reward_pool: AccountId,
        amm_contract: AccountId,
        admin: AccountId,
    }

    impl MiningPool {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(
            main_contract: AccountId,
            merchant_contract: AccountId,
            node_reward_pool: AccountId,
            amm_contract: AccountId
        ) -> Self {
            Self {
                main_contract,
                node_reward_pool,
                merchant_contract,
                amm_contract,
                admin: Self::env().caller(),
            }
        }

        #[ink(message, payable)]
        pub fn process_merchant_payment(&self) -> Result<(), Error> {
            let valid_caller = self.only_callable_by(self.merchant_contract);
            if let Err(e) = valid_caller {
                return Err(e);
            }
            let received_amount = self.env().transferred_value();
            let three_percent = Perbill::from_percent(3);
            let amount_to_pool = three_percent.mul_floor(received_amount);
            let _ = self.env().transfer(self.node_reward_pool, amount_to_pool);
            Ok(())
        }

        #[ink(message)]
        pub fn merchant_user_redeem_d9(
            &self,
            user_account: AccountId,
            redeemable_usdt: Balance
        ) -> Result<Balance, Error> {
            // let valid_caller = self.only_callable_by(self.merchant_contract);
            // if let Err(e) = valid_caller {
            //     return Err(e);
            // }
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
        pub fn change_node_reward_contract(
            &mut self,
            node_reward_contract: AccountId
        ) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            self.node_reward_pool = node_reward_contract;
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

        /// We test if the default constructor does its job.
        #[ink::test]
        fn default_works() {
            let mining_pool = MiningPool::default();
            assert_eq!(mining_pool.get(), false);
        }
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
