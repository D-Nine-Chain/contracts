#![cfg_attr(not(feature = "std"), no_std, no_main)]
use scale::{ Decode, Encode };

#[ink::contract]
mod node_reward {
    use super::*;
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        onlyCallableBy(AccountId),
    }
    #[ink(storage)]
    pub struct NodeReward {
        admin: AccountId,
        main: AccountId,
    }

    impl NodeReward {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, main: AccountId) -> Self {
            Self {
                admin,
                main,
            }
        }

        #[ink(message)]
        pub fn change_main(&mut self, contract_address: AccountId) -> Result<(), Error> {
            let caller = self.env().caller();
            if caller != self.admin {
                return Err(Error::onlyCallableBy(self.admin));
            }
            self.main = contract_address;
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
            let node_reward = NodeReward::default();
            assert_eq!(node_reward.get(), false);
        }
        //   #[ink::test]
        //   fn it_works() {
        //       let mut node_reward = NodeReward::new(false);
        //       assert_eq!(node_reward.get(), false);
        //       node_reward.flip();
        //       assert_eq!(node_reward.get(), true);
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
            let constructor = NodeRewardRef::default();

            // When
            let contract_account_id = client
                .instantiate("node_reward", &ink_e2e::alice(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            // Then
            let get = build_message::<NodeRewardRef>(contract_account_id.clone()).call(|node_reward|
                node_reward.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::alice(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            Ok(())
        }

        /// We test that we can read and write a value from the on-chain contract contract.
        #[ink_e2e::test]
        async fn it_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = NodeRewardRef::new(false);
            let contract_account_id = client
                .instantiate("node_reward", &ink_e2e::bob(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            let get = build_message::<NodeRewardRef>(contract_account_id.clone()).call(|node_reward|
                node_reward.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            // When
            let flip = build_message::<NodeRewardRef>(contract_account_id.clone()).call(
                |node_reward| node_reward.flip()
            );
            let _flip_result = client
                .call(&ink_e2e::bob(), flip, 0, None).await
                .expect("flip failed");

            // Then
            let get = build_message::<NodeRewardRef>(contract_account_id.clone()).call(|node_reward|
                node_reward.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), true));

            Ok(())
        }
    }
}
