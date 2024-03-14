#![cfg_attr(not(feature = "std"), no_std, no_main)]
/// calculate the share of session reward that is due to a particular node
/// session rewards are calculated as 10 percent of the accumulation of total burned tokens in the main pool
/// and total of d9 tokens processed by the merchant contract
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod node_reward {
    use super::*;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::prelude::vec::Vec;
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{ Decode, Encode };
    use sp_arithmetic::Perquintill;

    #[ink(storage)]
    pub struct NodeReward {
        admin: AccountId,
        new_admin: AccountId,
        mining_pool: AccountId,
        rewards_pallet: AccountId,
        ///reward pool for the session and amount paid in total
        session_rewards: Mapping<u32, (Balance, Balance)>,
        node_reward: Mapping<AccountId, Balance>,
        authorized_reward_receiver: Mapping<AccountId, AccountId>,
        /// minimum number of votes a node must have to receive a reward
        vote_limit: u64,
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
        BeyondQualificationForNodeStatus,
        ErrorIssuingPayment,
        ErrorGettingSessionPoolFromMiningPoolContract,
        NotAuthorizedToWithdraw,
        NothingToWithdraw,
        ErrorGettingCurrentValidators,
    }
    #[ink(event)]
    pub struct NodeRewardPaid {
        #[ink(topic)]
        node: AccountId,
        #[ink(topic)]
        receiver: AccountId,
        amount: Balance,
    }

    impl NodeReward {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(mining_pool: AccountId, rewards_pallet: AccountId) -> Self {
            Self {
                admin: Self::env().caller(),
                new_admin: [0u8; 32].into(),
                mining_pool,
                rewards_pallet,
                session_rewards: Mapping::new(),
                node_reward: Mapping::new(),
                authorized_reward_receiver: Mapping::new(),
                vote_limit: 680_000,
            }
        }

        fn only_callable_by(&self, account_id: AccountId) -> Result<(), Error> {
            if self.env().caller() != account_id {
                return Err(Error::OnlyCallableBy(account_id));
            }
            Ok(())
        }

        #[ink(message)]
        pub fn set_mining_pool(&mut self, mining_pool: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.mining_pool = mining_pool;
            Ok(())
        }

        #[ink(message)]
        pub fn set_rewards_pallet(&mut self, rewards_pallet: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.rewards_pallet = rewards_pallet;
            Ok(())
        }

        #[ink(message)]
        pub fn relinquish_admin(&mut self, new_admin: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.new_admin = new_admin;
            Ok(())
        }

        #[ink(message)]
        pub fn accept_admin(&mut self) -> Result<(), Error> {
            self.only_callable_by(self.new_admin)?;
            self.admin = self.new_admin;
            self.new_admin = [0u8; 32].into();
            Ok(())
        }

        #[ink(message)]
        pub fn cancel_admin_relinquish(&mut self) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.new_admin = [0u8; 32].into();
            Ok(())
        }
        #[ink(message)]
        pub fn get_vote_limit(&self) -> u64 {
            self.vote_limit
        }

        #[ink(message)]
        pub fn change_vote_limit(&mut self, new_limit: u64) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.vote_limit = new_limit;
            Ok(())
        }

        #[ink(message)]
        pub fn withdraw_reward(&mut self, node_id: AccountId) -> Result<(), Error> {
            let caller = self.env().caller();
            let _ = self.validate_withdraw(node_id, caller)?;
            let reward_balance = self.node_reward.get(&node_id).unwrap_or(0);
            if reward_balance == 0 {
                return Err(Error::NothingToWithdraw);
            }
            let payment_request_result = self.tell_mining_pool_to_pay(caller, reward_balance);
            if payment_request_result.is_err() {
                return Err(Error::ErrorIssuingPayment);
            }
            let _ = self.deduct_node_reward(node_id)?;
            self.env().emit_event(NodeRewardPaid {
                node: node_id,
                receiver: caller,
                amount: reward_balance,
            });
            Ok(())
        }

        #[ink(message)]
        pub fn get_session_rewards_data(&self, session_index: u32) -> Option<(Balance, Balance)> {
            self.session_rewards.get(&session_index)
        }

        #[ink(message)]
        pub fn get_node_reward_data(&self, node_id: AccountId) -> Option<Balance> {
            self.node_reward.get(node_id)
        }

        #[ink(message)]
        pub fn get_authorized_receiver(&self, node_id: AccountId) -> AccountId {
            match self.authorized_reward_receiver.get(node_id) {
                Some(receiver) => receiver,
                None => node_id,
            }
        }

        #[ink(message)]
        pub fn set_authorized_receiver(
            &mut self,
            node_id: AccountId,
            receiver: AccountId
        ) -> Result<(), Error> {
            self.only_callable_by(node_id)?;
            self.authorized_reward_receiver.insert(node_id, &receiver);
            Ok(())
        }

        #[ink(message)]
        pub fn remove_authorized_receiver(&mut self, node_id: AccountId) -> Result<(), Error> {
            self.only_callable_by(node_id)?;
            self.authorized_reward_receiver.remove(node_id);
            Ok(())
        }

        #[ink(message)]
        pub fn update_rewards(
            &mut self,
            last_session: u32,
            sorted_nodes_and_votes: Vec<(AccountId, u64)>
        ) -> Result<(), Error> {
            self.only_callable_by(self.rewards_pallet)?;
            let mut nodes_and_votes_vec: Vec<(AccountId, u64)> = sorted_nodes_and_votes.clone();
            // let current_active_validators = self.get_active_validators()?;
            let mut total_paid_out: Balance = 0;
            let reward_pool = self.get_reward_pool(last_session)?;
            // from pallet it is truncated to limit of MaxCandidates
            // here we truncate to max payable of 288
            if nodes_and_votes_vec.len() > 288 {
                nodes_and_votes_vec.truncate(288);
            }
            for (index, node_and_votes) in nodes_and_votes_vec.iter().enumerate() {
                let get_node_tier_result = self.node_tier_by_vec_position(index);
                if get_node_tier_result.is_err() {
                    continue;
                }
                let node_tier = get_node_tier_result.unwrap();
                let node_share = self.calc_single_node_share(reward_pool, node_tier);

                if node_and_votes.1 >= self.vote_limit {
                    let node_id: AccountId = node_and_votes.0;
                    let _ = self.credit_node_reward(node_id, node_share)?;
                    total_paid_out = total_paid_out.saturating_add(node_share);
                    let _ = self.deduct_from_reward_pool(node_share);
                }
            }
            self.session_rewards.insert(last_session, &(reward_pool, total_paid_out));
            Ok(())
        }

        fn validate_withdraw(&self, node_id: AccountId, requester: AccountId) -> Result<(), Error> {
            let authorized_receiver = self.authorized_reward_receiver.get(&node_id);
            match authorized_receiver {
                Some(authorized_receiver) => {
                    if authorized_receiver != requester {
                        return Err(Error::NotAuthorizedToWithdraw);
                    }
                }
                None => {
                    if requester != node_id {
                        return Err(Error::NotAuthorizedToWithdraw);
                    }
                }
            }
            Ok(())
        }

        // fn get_active_validators(&self) -> Result<Vec<AccountId>, Error> {
        //     let retrieve_validators_result = self.env().extension().get_active_validators();
        //     match retrieve_validators_result {
        //         Ok(validators) => Ok(validators),
        //         Err(_) => Err(Error::ErrorGettingCurrentValidators),
        //     }
        // }

        fn credit_node_reward(
            &mut self,
            node_id: AccountId,
            balance_increase: Balance
        ) -> Result<(), Error> {
            let node_reward_balance: Balance = self.node_reward.get(&node_id).unwrap_or(0);
            let new_balance: Balance = node_reward_balance.saturating_add(balance_increase);
            self.node_reward.insert(node_id, &new_balance);
            Ok(())
        }

        fn deduct_from_reward_pool(&self, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("deduct_from_reward_pool"))
                    ).push_arg(amount)
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        fn deduct_node_reward(&mut self, node_id: AccountId) -> Result<(), Error> {
            self.node_reward.insert(node_id, &0);
            Ok(())
        }

        fn tell_mining_pool_to_pay(
            &self,
            receiver: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("pay_node_reward")))
                        .push_arg(receiver)
                        .push_arg(amount)
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        fn get_reward_pool(&self, session_index: u32) -> Result<Balance, Error> {
            let result = build_call::<D9Environment>()
                .call(self.mining_pool)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("update_pool_and_retrieve"))
                    ).push_arg(session_index)
                )
                .returns::<Result<Balance, Error>>()
                .invoke();
            if result.is_err() {
                return Err(Error::ErrorGettingSessionPoolFromMiningPoolContract);
            }
            Ok(result.unwrap())
        }

        /// determine the rank of a node with respect to the session and other nodes
        fn node_tier_by_vec_position(&self, index: usize) -> Result<NodeTier, Error> {
            if (0..9).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Upper))
            } else if (9..18).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Middle))
            } else if (18..27).contains(&index) {
                Ok(NodeTier::Super(SuperNodeSubTier::Lower))
            } else if (27..127).contains(&index) {
                Ok(NodeTier::StandBy)
            } else if (127..288).contains(&index) {
                Ok(NodeTier::Candidate)
            } else {
                Err(Error::BeyondQualificationForNodeStatus)
            }
        }

        fn calc_single_node_share(&self, reward_pool: Balance, node_tier: NodeTier) -> Balance {
            let node_percent = match node_tier {
                NodeTier::Super(super_node_sub_tier) => {
                    let percent = match super_node_sub_tier {
                        SuperNodeSubTier::Upper => 3,
                        SuperNodeSubTier::Middle => 2,
                        SuperNodeSubTier::Lower => 1,
                    };
                    Perquintill::from_percent(percent)
                }
                NodeTier::StandBy => Perquintill::from_rational(3u64, 1000u64),
                NodeTier::Candidate => Perquintill::from_rational(1u64, 1000u64),
            };

            node_percent.mul_floor(reward_pool)
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
