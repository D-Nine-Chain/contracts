#![cfg_attr(not(feature = "std"), no_std, no_main)]
/// calculate the share of session reward that is due to a particular node
/// session rewards are calculated as 10 percent of the accumulation of total burned tokens in the main pool
/// and total of d9 tokens processed by the merchant contract
use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod node_reward {
    use core::result;

    use super::*;
    use ink::primitives::AccountId;
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;
    use scale_info::build;
    use sp_arithmetic::Perbill;
    #[ink(storage)]
    pub struct NodeReward {
        admin: AccountId,
        new_admin: AccountId,
        mining_pool: AccountId,
        rewards_pallet: AccountId,
    }
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct RewardPayments {
        receiver: AccountId,
        amount: Balance,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct TierRewardPools {
        supers: Balance,
        standbys: Balance,
        candidates: Balance,
    }

    // impl TierRewardPools {
    //     fn calc_single_node_share(&self, node_tier: NodeTier, percent: Perbill) -> Balance {
    //         let allotment = match node_tier {
    //             NodeTier::Super(_) => self.supers,
    //             NodeTier::StandBy => self.standbys,
    //             NodeTier::Candidate => self.candidates,
    //         };
    //         let payment = percent.mul_floor(allotment);
    //         payment
    //     }
    // }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum NodeTier {
        Super(SuperNodeSubTier),
        StandBy,
        Candidate,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum SuperNodeSubTier {
        Upper,
        Middle,
        Lower,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        OnlyCallableBy(AccountId),
        IssueGettingValidatorsOrCandidates,
        ErrorGettingSession,
        PaymentWouldExceedAllotment,
        NotASuperNode,
        NotAValidNode,
        BeyondQualificationForNodeStatus,
        ErrorIssuingPayment,
        RewardReceivedThisSession,
        ErrorGettingSessionList,
        ErrorGettingUserSupportedNodes,
        UserDoesntSupportAnyNodes,
        CallerNotNodeController,
        IssuingDeterminingPayout,
        ErrorGettingNodeSharingPercentage,
        ErrorGettingMiningPool,
        ErrorGettingSessionPoolFromMiningPoolContract,
    }

    impl NodeReward {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(mining_pool: AccountId, rewards_pallet: AccountId) -> Self {
            Self {
                admin: Self::env().caller(),
                new_admin: AccountId::default(),
                mining_pool,
                rewards_pallet,
            }
        }
        #[ink(message)]
        pub fn set_mining_pool(&mut self, mining_pool: AccountId) {
            self.only_callable_by(self.admin)?;
            self.mining_pool = mining_pool;
        }

        #[ink(message)]
        pub fn set_rewards_pallet(&mut self, rewards_pallet: AccountId) {
            self.only_callable_by(self.admin)?;
            self.rewards_pallet = rewards_pallet;
        }

        #[ink(message)]
        pub fn relinquish_admin(&mut self, new_admin: AccountId) {
            self.only_callable_by(self.admin)?;
            self.new_admin = new_admin;
        }

        #[ink(message)]
        pub fn accept_admin(&mut self) {
            self.only_callable_by(self.new_admin)?;
            self.admin = self.new_admin;
            self.new_admin = AccountId::default();
        }

        #[ink(message)]
        pub fn cancel_admin_relinquish(&mut self) {
            self.only_callable_by(self.admin)?;
            self.new_admin = AccountId::default();
        }

        #[ink(message)]
        pub fn issue_payments(
            &mut self,
            last_session: u32,
            supported_nodes: Vec<AccountId>
        ) -> Result<(), Error> {
            self.only_callable_by(self.rewards_pallet)?;
            let rewards_by_tier = self.get_tier_session_rewards(last_session)?;
            Ok(())
        }
        fn get_total_session_pool(&self, session_index: u32) -> Result<Balance, Error> {
            let result = build_call()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("calculate_session_pool"))
                    ).push_arg(session_index)
                )
                .returns::<Result<Balance, Error>>()
                .try_invoke();
            if result.is_err() {
                return Err(Error::ErrorGettingSessionPoolFromMiningPoolContract);
            }
            Ok(result.unwrap())
        }

        // fn tier_allotments(&self,

        fn tier_session_allotment(&self, tier: NodeTier) -> Result<Balance, Error> {
            let tier_allotment = self.get_tier_reward_allotments(session_index)?;

            let tier_percent = self.get_percent_by_tier(tier)?;

            let node_share = tier_allotment.calc_single_node_share(tier, tier_percent);

            Ok(node_share)
        }

        /// determine the rank of a node with respect to the session and other nodes
        fn node_tier_by_vec_position(&self, sorted_vec_index: usize) -> Result<NodeTier, Error> {
            if (0..8).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Upper))
            } else if (9..17).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Middle))
            } else if (18..26).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Lower))
            } else if (27..126).contains(&index) {
                Ok(NodeTier::StandBy)
            } else if (127..287).contains(&index) {
                Ok(NodeTier::Candidate)
            } else {
                Err(Error::BeyondQualificationForNodeStatus)
            }
        }

        /// get the session record for the session previous to the current session
        ///
        /// the session record contains the amount of D9 tokens to be paid out to each node tier
        fn get_tier_session_rewards(
            &self,
            session_index: u32
        ) -> Result<(u32, TierRewardPools), Error> {
            let total_session_reward_pool = self.get_total_session_pool(session_index)?;

            let session_reward_allotment = self.calculate_session_total_allotment();
            let supers_percent = Perbill::from_percent(54);
            let standbys_percent = Perbill::from_percent(30);
            let candidates_percent = Perbill::from_percent(16);
            let session_reward = TierRewardPools {
                supers: supers_percent.mul_floor(session_reward_allotment),
                standbys: standbys_percent.mul_floor(session_reward_allotment),
                candidates: candidates_percent.mul_floor(session_reward_allotment),
            };
            session_reward;

            Ok((session_index, session_reward))
        }

        /// determine the percent of the Tiered allotment that a node should receive .e.g  54% of session rewards go to super nodes and  a Upper super node receives 3% of that 54%
        #[ink(message)]
        pub fn get_percent_by_tier(&self, node_tier: NodeTier) -> Result<Perbill, Error> {
            match node_tier {
                NodeTier::Super(super_node_sub_tier) => {
                    let percent = match super_node_sub_tier {
                        SuperNodeSubTier::Upper => 3,
                        SuperNodeSubTier::Middle => 2,
                        SuperNodeSubTier::Lower => 1,
                    };
                    Ok(Perbill::from_percent(percent))
                }
                NodeTier::StandBy => Ok(Perbill::from_rational(3u32, 1000u32)),
                NodeTier::Candidate => Ok(Perbill::from_rational(1u32, 1000u32)),
            }
        }

        /// calculates the reward per session using the total burned in the main pool
        fn calculate_session_total_allotment(&self) -> Balance {
            let pool_balance = self.env().balance();
            let ten_percent = Perbill::from_percent(10);
            ten_percent.mul_floor(pool_balance)
        }

        fn only_callable_by(&self, account_id: AccountId) -> Result<(), Error> {
            if self.env().caller() != account_id {
                return Err(Error::OnlyCallableBy(account_id));
            }
            Ok(())
        }

        ///calculate the share of session reward that is due to a particular node
        ///
        /// the value does not include deductions from percentage split with supporters
        /// this function is also used to calculate supporter share
        fn calc_node_session_share(
            &self,
            sorted_vec_index: usize,
            node_id: AccountId
        ) -> Result<Balance, Error> {
            let node_tier = self.node_tier_by_vec_position(sorted_vec_index)?;

            let tier_allotment_request = self.get_tier_reward_allotments(session_index);
            if let Err(e) = tier_allotment_request {
                return Err(e);
            }
            let tier_allotment = tier_allotment_request.unwrap();

            let node_tier_percent_result = self.get_percent_by_tier(node_tier);
            if let Err(e) = node_tier_percent_result {
                return Err(e);
            }
            let node_tier_percent = node_tier_percent_result.unwrap();
            let node_session_share = tier_allotment.calc_single_node_share(node_tier, percent);

            Ok(node_session_share)
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
