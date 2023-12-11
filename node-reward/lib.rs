#![cfg_attr(not(feature = "std"), no_std, no_main)]
use scale::{ Decode, Encode };
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod node_reward {
    use super::*;
    use ink::storage::Mapping;
    use ink::prelude::vec::Vec;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;
    use sp_arithmetic::Perbill;
    #[ink(storage)]
    pub struct NodeReward {
        admin: AccountId,
        main: AccountId,
        session_rewards: Mapping<u32, SessionReward>,
        last_session_payments: Mapping<AccountId, u32>,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct SessionReward {
        supers: Balance,
        standbys: Balance,
        candidates: Balance,
    }

    impl SessionReward {
        fn calc_payment(&self, node_tier: NodeTier, percent: Perbill) -> Balance {
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
    }

    impl NodeReward {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, main: AccountId) -> Self {
            Self {
                admin,
                main,
                session_rewards: Mapping::new(),
                last_session_payments: Mapping::new(),
            }
        }

        #[ink(message)]
        pub fn get_session_record(&self, session_index: u32) -> Option<SessionReward> {
            let session_reward_option = self.session_rewards.get(&session_index);
            session_reward_option
        }

        #[ink(message)]
        pub fn request_payment(&mut self) -> Result<Balance, Error> {
            let node_id = self.env().caller();
            let tier_result = self.determine_node_tier(node_id);
            if let Err(e) = tier_result {
                return Err(e);
            }

            let node_tier = tier_result.unwrap();

            let payment_result = self.issue_node_payment(node_id, node_tier);
            if payment_result.is_err() {
                return Err(Error::ErrorIssuingPayment);
            }

            Ok(payment_result.unwrap())
        }

        /// determine the rank of a node
        fn determine_node_tier(&self, account_id: AccountId) -> Result<NodeTier, Error> {
            let validators = self.get_validators();
            let candidates = self.get_candidates();

            if validators.contains(&account_id) {
                match validators.iter().position(|&x| x == account_id) {
                    Some(index) => {
                        if (0..9).contains(&index) {
                            Ok(NodeTier::Super(SuperNodeSubTier::Upper))
                        } else if (10..18).contains(&index) {
                            Ok(NodeTier::Super(SuperNodeSubTier::Middle))
                        } else if (19..27).contains(&index) {
                            Ok(NodeTier::Super(SuperNodeSubTier::Lower))
                        } else {
                            Err(Error::BeyondQualificationForNodeStatus)
                        }
                    }
                    None => Err(Error::NotAValidNode),
                }
            } else if candidates.contains(&account_id) {
                match candidates.iter().position(|&x| x == account_id) {
                    Some(index) => {
                        if (0..99).contains(&index) {
                            Ok(NodeTier::StandBy)
                        } else if (100..260).contains(&index) {
                            Ok(NodeTier::Candidate)
                        } else {
                            Err(Error::NotASuperNode)
                        }
                    }
                    None => Err(Error::NotASuperNode),
                }
            } else {
                Err(Error::NotAValidNode)
            }
        }

        fn issue_node_payment(
            &mut self,
            node_id: AccountId,
            node_tier: NodeTier
        ) -> Result<Balance, Error> {
            let session_record_result = self.get_payout_session_record();
            if let Err(e) = session_record_result {
                return Err(e);
            }
            let (session_index, session_reward) = session_record_result.unwrap();
            let last_payment_option = self.last_session_payments.get(node_id);
            let last_payment_session = match last_payment_option {
                Some(last_payment) => last_payment,
                None => 0,
            };
            if last_payment_session == session_index {
                return Err(Error::RewardReceivedThisSession);
            }

            let payout_percent_result = self.determine_payout_percent(node_tier);
            if let Err(e) = payout_percent_result {
                return Err(e);
            }
            let payout_percent = payout_percent_result.unwrap();
            let payment_amount = session_reward.calc_payment(node_tier, payout_percent);

            let payment_result = self.request_payment_from_main(node_id, payment_amount);
            if payment_result.is_err() {
                return Err(Error::ErrorIssuingPayment);
            }

            Ok(payment_amount)
        }

        fn request_payment_from_main(
            &self,
            node_id: AccountId,
            payment_amount: Balance
        ) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.main)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("pay_node_reward")))
                        .push_arg(node_id)
                        .push_arg(payment_amount)
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        /// determine the percent payout based on a node's status
        #[ink(message)]
        pub fn determine_payout_percent(&self, node_tier: NodeTier) -> Result<Perbill, Error> {
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

        fn get_payout_session_record(&self) -> Result<(u32, SessionReward), Error> {
            let session_index_result = self.get_payout_session();
            if let Err(e) = session_index_result {
                return Err(e);
            }

            let session_index = session_index_result.unwrap();

            let session_reward_option = self.session_rewards.get(session_index);
            let session_reward = match session_reward_option {
                Some(session_reward) => session_reward,
                None => {
                    let session_reward_allotment = self.calculate_session_allotment();
                    let supers_percent = Perbill::from_percent(54);
                    let standbys_percent = Perbill::from_percent(30);
                    let candidates_percent = Perbill::from_percent(16);
                    let session_reward = SessionReward {
                        supers: supers_percent.mul_floor(session_reward_allotment),
                        standbys: standbys_percent.mul_floor(session_reward_allotment),
                        candidates: candidates_percent.mul_floor(session_reward_allotment),
                    };
                    session_reward
                }
            };

            Ok((session_index, session_reward))
        }

        fn get_payout_session(&self) -> Result<u32, Error> {
            let session_result = self.env().extension().get_current_session();
            if session_result.is_err() {
                return Err(Error::ErrorGettingSession);
            }
            let session = session_result.unwrap();
            Ok(session.saturating_sub(1))
        }

        /// calculates the reward per session using the total burned in the main pool
        fn calculate_session_allotment(&self) -> Balance {
            let main_balance = self.get_balance_from_main();
            let ten_percent = Perbill::from_percent(10);
            ten_percent.mul_floor(main_balance)
        }
        fn get_balance_from_main(&self) -> Balance {
            build_call::<D9Environment>()
                .call(self.main)
                .gas_limit(0) // replace with an appropriate gas limit
                .exec_input(ExecutionInput::new(Selector::new(ink::selector_bytes!("get_balance"))))
                .returns::<Balance>()
                .invoke()
        }

        fn get_validators(&self) -> Vec<AccountId> {
            let validators_result = self.env().extension().get_validators();
            let validators = match validators_result {
                Ok(validators) => validators,
                Err(_) => Vec::new(),
            };
            validators
        }

        fn get_candidates(&self) -> Vec<AccountId> {
            let candidates = self.env().extension().get_candidates();
            let candidates = match candidates {
                Ok(candidates) => candidates,
                Err(_) => Vec::new(),
            };
            candidates
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
