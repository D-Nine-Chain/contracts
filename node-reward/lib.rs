#![cfg_attr(not(feature = "std"), no_std, no_main)]
use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod node_reward {
    use core::result;

    use super::*;
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;
    use scale_info::build;
    use sp_arithmetic::Perbill;
    #[ink(storage)]
    pub struct NodeReward {
        admin: AccountId,
        main: AccountId,
        mining_pool: AccountId,
        node_surrogates: Mapping<AccountId, AccountId>,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct TierAllotments {
        supers: Balance,
        standbys: Balance,
        candidates: Balance,
    }

    impl TierAllotments {
        fn calc_single_node_share(&self, node_tier: NodeTier, percent: Perbill) -> Balance {
            let allotment = match node_tier {
                NodeTier::Super(_) => self.supers,
                NodeTier::StandBy => self.standbys,
                NodeTier::Candidate => self.candidates,
            };
            let payment = percent.mul_floor(allotment);
            payment
        }
    }

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
    }

    impl NodeReward {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, main: AccountId) -> Self {
            Self {
                admin,
                main,
                mining_pool,
                node_surrogates: Mapping::new(),
            }
        }

        /// a node controller is the account that is allowed to call certain functions on behalf of a node
        #[ink::message]
        pub fn define_node_surrogate(&self, controller_id: AccountId) -> () {
            let node_id = self.env().caller();
            self.node_surrogates.insert(node_id, &controller_id);
        }

        #[ink(message)]
        pub fn get_tier_reward_allotments(&self, session_index: u32) -> Option<TierAllotments> {
            let tier_allotment_opt = self.tier_allotments_by_session_index.get(&session_index);
            tier_allotment_opt
        }

        #[ink(message)]
        pub fn node_payment_request(&mut self) -> Result<Balance, Error> {
            let node_id = self.env().caller();
            if let Err(result) = self.validate_node_session_payment(node_id) {
                return result;
            }
            let payout_index = match self.get_payout_session_index() {
                Ok(session_index) => session_index,
                Err(e) => {
                    return Err(e);
                }
            };
            let session_share = match self.calc_node_session_share(payout_index, node_id) {
                Ok(session_share) => session_share,
                Err(e) => {
                    return Err(e);
                }
            };
            //the split between node and supporters
            let node_share = match self.get_node_sharing_percentage(node_id) {
                Ok(node_share) => node_share,
                Err(e) => {
                    return Err(e);
                }
            };
        }

        #[ink(message)]
        pub fn get_mining_pool_total(&self) -> Result<Balance, Error> {
            self.get_mining_pool_total_from_contract()
        }

        #[ink(message)]
        pub fn get_node_pool_total(&self) -> Balance {
            self.env().balance()
        }

        fn get_mining_pool_total_from_contract(&self) -> Result<Balance, Error> {
            let result = build_call()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("get_mining_pool_total")))
                        .push_arg(user_account)
                        .push_arg(redeemable_usdt)
                )
                .returns::<Balance>()
                .try_invoke()?;
            if result.is_err() {
                return Err(Error::ErrorGettingMiningPool);
            }
            Ok(result.unwrap())
        }

        /// the same as `node_request_payment` but the node controller is the one calling the function
        #[ink(message)]
        pub fn surrogate_payment_request(&mut self, node_id: AccountId) -> Result<Balance, Error> {
            let caller = self.env().caller();
            let surrogate_check_result = self.check_surrogate(caller, node_id);
            if let Err(e) = surrogate_check_result {
                return Err(e);
            }

            let payment_result = self.process_node_payment(node_id, caller);
            payment_result
        }

        #[ink(message)]
        pub fn supporter_payment_request(&mut self) -> Result<Balance, Error> {
            let supporter_id = self.env().caller();
            let supported_nodes_result = self
                .env()
                .extension()
                .get_user_supported_nodes(supporter_id);

            if supported_nodes_result.is_err() {
                return Err(Error::ErrorGettingUserSupportedNodes);
            }

            let supported_nodes = supported_nodes_result.unwrap();
            if supported_nodes.len() == 0 {
                return Err(Error::UserDoesntSupportAnyNodes);
            }
            let payable_nodes = self.determine_payable_user_supported_nodes(supported_nodes);
            let session_index_result = self.get_payout_session_index();
            if let Err(e) = session_index_result {
                return Err(e);
            }
            let session_index = session_index_result.unwrap();
            let session_record_result = self.get_tier_reward_allotments(session_index);
            if let Err(e) = session_record_result {
                return Err(e);
            }
            let (session_index, session_reward) = session_record_result.unwrap();
            let last_payment_option = self.last_session_payment_index.get(supporter_id);
            let last_payment_session = match last_payment_option {
                Some(last_payment) => last_payment,
                None => 0,
            };
            if last_payment_session == session_index {
                return Err(Error::RewardReceivedThisSession);
            }

            let payment_amount = session_reward.standbys / 10;

            let payment_result = self.request_supporter_payment_from_main(
                supporter_id,
                payment_amount
            );
            if payment_result.is_err() {
                return Err(Error::ErrorIssuingPayment);
            }

            Ok(payment_amount)
        }

        fn check_surrogate(&self, caller: AccountId, node_id: AccountId) -> Result<(), Error> {
            let controller_result = self.node_controllers.get(&node_id);
            match controller_result {
                Some(controller) => {
                    if caller != controller {
                        return Err(Error::CallerNotNodeController);
                    }
                }
                None => {
                    return Err(Error::CallerNotNodeController);
                }
            }
            Ok(())
        }

        ///calculate the share of session reward that is due to a particular node
        ///
        /// the value does not include deductions from percentage split with supporters
        /// this function is also used to calculate supporter share
        fn calc_node_session_share(
            &self,
            session_index: u32,
            node_id: AccountId
        ) -> Result<Balance, Error> {
            let tier_inquiry_result = self.determine_node_tier(node_id, session_index);
            if let Err(e) = tier_inquiry_result {
                return Err(e);
            }
            let node_tier = tier_inquiry_result.unwrap();

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

        /// determine the rank of a node with respect to the session and other nodes
        fn determine_node_tier(
            &self,
            account_id: AccountId,
            session_index: u32
        ) -> Result<NodeTier, Error> {
            let session_list_result: Result<Vec<AccountId>, _> = self
                .env()
                .extension()
                .get_session_node_list(session_index);
            if session_list_result.is_err() {
                return Err(Error::ErrorGettingSessionList);
            }
            let session_list = session_list_result.unwrap();

            match session_list.iter().position(|&x| x == account_id) {
                Some(index) => {
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
                None => Err(Error::NotAValidNode),
            }
        }

        /// validate that a node can payout to either a surrogate or itself
        fn validate_node_session_payment(&self, node_id: AccountId) -> Result<(), Error> {
            let last_paid_session = match self.last_paid_session.get(node_id) {
                Some(last_paid_session) => last_paid_session,
                None => 0,
            };
            let current_payout_session = match self.get_payout_session_index() {
                Ok(session_index) => session_index,
                Err(e) => {
                    return Err(e);
                }
            };
            if current_payout_session == last_paid_session {
                return Err(Error::RewardReceivedThisSession);
            }
            let node_list = match
                self.env().extension().get_session_node_list(current_payout_session)
            {
                Ok(node_list) => node_list,
                Err(e) => {
                    return Err(Error::ErrorGettingSessionList);
                }
            };
            if node_list.contains(&node_id) {
                return Err(Error::NotAValidNode);
            }
            Ok(())
        }

        fn determine_payable_user_supported_nodes(
            &self,
            supported_nodes: Vec<AccountId>
        ) -> Vec<AccountId> {}

        fn issue_payment(
            &self,
            payee: AccountId,
            payment_amount: Balance
        ) -> Result<Balance, Error> {
            let payment_result = self.env().transfer(payee, payment_amount);
            if payment_result.is_err() {
                return Err(Error::ErrorIssuingPayment);
            }
            Ok(payment_amount)
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
        /// get the session record for the session previous to the current session
        ///
        /// the session record contains the amount of D9 tokens to be paid out to each node tier
        fn get_tier_allotments(&self) -> Result<(u32, TierAllotments), Error> {
            let session_index_result = self.get_payout_session_index();
            if let Err(e) = session_index_result {
                return Err(e);
            }
            let session_index = session_index_result.unwrap();

            let session_reward_option = self.tier_allotments_by_session_index.get(session_index);
            let session_reward = match session_reward_option {
                Some(session_reward) => session_reward,
                None => {
                    let session_reward_allotment = self.calculate_session_total_allotment();
                    let supers_percent = Perbill::from_percent(54);
                    let standbys_percent = Perbill::from_percent(30);
                    let candidates_percent = Perbill::from_percent(16);
                    let session_reward = TierAllotments {
                        supers: supers_percent.mul_floor(session_reward_allotment),
                        standbys: standbys_percent.mul_floor(session_reward_allotment),
                        candidates: candidates_percent.mul_floor(session_reward_allotment),
                    };
                    session_reward
                }
            };

            Ok((session_index, session_reward))
        }

        fn get_payout_session_index(&self) -> Result<u32, Error> {
            let session_result = self.env().extension().get_current_session_index();
            if session_result.is_err() {
                return Err(Error::ErrorGettingSession);
            }
            let session = session_result.unwrap();
            Ok(session.saturating_sub(1))
        }

        /// calculates the reward per session using the total burned in the main pool
        fn calculate_session_total_allotment(&self) -> Balance {
            let pool_balance = self.env().balance();
            let ten_percent = Perbill::from_percent(10);
            ten_percent.mul_floor(pool_balance)
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
