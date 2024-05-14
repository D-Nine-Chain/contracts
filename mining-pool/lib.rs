#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use d9_chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod mining_pool {
    use super::*;
    use ink::env::call::{ build_call, ExecutionInput, Selector };
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{ Decode, Encode };
    use sp_arithmetic::Perquintill;
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
        ErrorAddingVotes
    }

    #[ink(storage)]
    pub struct MiningPool {
        /// contract admin
        admin: AccountId,
        /// main contract that holds burn data and burn funds
        main_contract: AccountId,
        /// merchant contract, its funds are sent here
        merchant_contract: AccountId,
        /// contract that defines node rewards
        node_reward_contract: AccountId,
        /// decentralized exchange
        amm_contract: AccountId,
        /// total number of tokens processed by the merchant contract
        merchant_volume: Balance,
        /// the total number of tokens processed by merchant/burn contract at each recorded session
        volume_at_index: Mapping<u32, Balance>,
        /// last session index process by this contract by `node_reward_contract`
        last_session: u32,
        /// total accumulative reward session pool
        accumulative_reward_pool: Balance,
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
                merchant_volume: 0,
                volume_at_index: Mapping::new(),
                last_session: 0,
                accumulative_reward_pool: 0,
            }
        }
 
        #[ink(message)]
        pub fn get_accumulative_reward_pool(&self) -> Balance {
            self.accumulative_reward_pool
        }

        #[ink(message)]
        pub fn pay_node_reward(
            &mut self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let _ = self.only_callable_by(self.node_reward_contract)?;
            let _ = self.env().transfer(account_id, amount);
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_sub(amount);
            Ok(())
        }

        #[ink(message)]
        pub fn get_merchant_volume(&self) -> Balance {
            self.merchant_volume
        }

        #[ink(message)]
        pub fn get_session_volume(&self, session_index: u32) -> Balance {
            self.volume_at_index.get(&session_index).unwrap_or(0)
        }

        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, session_index: u32) -> Result<Balance, Error> {
            self.only_callable_by(self.node_reward_contract)?;

            self.last_session = session_index;
            let total_volume = self.get_total_volume();
            self.volume_at_index.insert(session_index, &total_volume);

            let session_delta = self.calculate_session_delta(session_index, total_volume)?;
            let three_percent: Perquintill = Perquintill::from_percent(3);
            let three_percent_of_delta = three_percent.mul_floor(session_delta);
            self.accumulative_reward_pool =
                self.accumulative_reward_pool.saturating_add(three_percent_of_delta);
            let ten_percent = Perquintill::from_percent(10);
            let reward_pool = ten_percent.mul_floor(self.accumulative_reward_pool);
            Ok(reward_pool)
        }

        #[ink(message)]
        pub fn deduct_from_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            let _ = self.only_callable_by(self.node_reward_contract)?;
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_sub(amount);
            Ok(())
        }

        fn calculate_session_delta(
            &self,
            session_index: u32,
            current_volume: Balance
        ) -> Result<Balance, Error> {
            let previous_index = self.get_previous_valid_session_index(session_index);
            let previous_volume = self.volume_at_index.get(&previous_index).unwrap_or(0);
            let session_delta = current_volume.saturating_sub(previous_volume);
            Ok(session_delta)
        }

        fn get_previous_valid_session_index(&self, last_session: u32) -> u32 {
            let mut previous_index = last_session.saturating_sub(1);
            while previous_index > 0 && self.volume_at_index.get(&previous_index).is_none() {
                previous_index = previous_index.saturating_sub(1);
            }
            previous_index
        }

        #[ink(message)]
        pub fn get_total_volume(&self) -> Balance {
            let total_burned = self.get_total_burned();
            let total_merchant_mined = self.merchant_volume;
            total_burned.saturating_add(total_merchant_mined)
        }

        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self, merchant_id:AccountId) -> Result<(), Error> {
            let _ = self.only_callable_by(self.merchant_contract)?;
            let received_amount = self.env().transferred_value();
            self.merchant_volume = self.merchant_volume.saturating_add(received_amount);
            
            // give merchant votes
            let votes = self.calc_votes_from_d9(received_amount);
            let add_vote_result = self.env().extension().add_voting_interests(merchant_id, votes);
            if add_vote_result.is_err() {
                return Err(Error::ErrorAddingVotes);
            }
            Ok(())
        }

        fn calc_votes_from_d9(&self, d9_amount:Balance)->u64{
            let one_d9:Balance = 1_000_000_000_000;
            let votes = d9_amount.saturating_div(one_d9);
            votes as u64
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
        pub fn send_to(&mut self, to: AccountId, amount: Balance) -> Result<(), Error> {
            let _ = self.only_callable_by(self.admin);
            let _ = self.env().transfer(to, amount);
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
