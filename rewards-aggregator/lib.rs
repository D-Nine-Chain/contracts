#![cfg_attr(not(feature = "std"), no_std, no_main)]

pub use chain_extension::D9Environment;

#[ink::contract(env = D9Environment)]
mod rewards_aggregator {
    use super::*;
    use ink::env::call::{build_call, ExecutionInput, Selector};
    use ink::selector_bytes;
    use ink::storage::Mapping;
    use scale::{Decode, Encode};
    use sp_arithmetic::Perquintill;
    // use substrate_fixed::{ FixedU128, types::extra::U12 };
    // type FixedBalance = FixedU128<U12>;

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Currency {
        D9,
        Usdt,
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
        AddingVotes,
        RedeemableUSDTZero,
    }

    #[ink(storage)]
    pub struct RewardsAggregator {
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

    impl RewardsAggregator {
        const PRICE_STORAGE_KEY: u32 = 999_999_999;
        const PRICE_PRECISION: Balance = 1_000_000;
        const PERCENT_PROTECT: Balance = 70;
        #[ink(constructor)]
        pub fn new(
            main_contract: AccountId,
            merchant_contract: AccountId,
            node_reward_contract: AccountId,
            amm_contract: AccountId,
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

        // ========== Getter Messages ==========
        #[ink(message)]
        pub fn get_accumulative_reward_pool(&self) -> Balance {
            self.accumulative_reward_pool
        }

        #[ink(message)]
        pub fn get_merchant_volume(&self) -> Balance {
            self.merchant_volume
        }

        #[ink(message)]
        pub fn get_session_volume(&self, session_index: u32) -> Balance {
            self.volume_at_index.get(session_index).unwrap_or(0)
        }

        #[ink(message)]
        pub fn get_total_volume(&self) -> Balance {
            let total_burned = self.get_total_burned();
            let total_merchant_mined = self.merchant_volume;
            total_burned.saturating_add(total_merchant_mined)
        }

        #[ink(message)]
        pub fn get_price_protection_info(&self) -> (Balance, Balance) {
            let highest = self.get_highest_price();
            let min_protected = highest
                .saturating_mul(Self::PERCENT_PROTECT)
                .saturating_div(100);
            (highest, min_protected)
        }

        // ========== Pool Operations Messages ==========
        #[ink(message)]
        pub fn update_pool_and_retrieve(&mut self, session_index: u32) -> Result<Balance, Error> {
            self.only_callable_by(self.node_reward_contract)?;

            self.last_session = session_index;
            let total_volume = self.get_total_volume();
            self.volume_at_index.insert(session_index, &total_volume);

            let session_delta = self.calculate_session_delta(session_index, total_volume)?;
            let three_percent: Perquintill = Perquintill::from_percent(3);
            let three_percent_of_delta = three_percent.mul_floor(session_delta);
            self.accumulative_reward_pool = self
                .accumulative_reward_pool
                .saturating_add(three_percent_of_delta);
            let ten_percent = Perquintill::from_percent(10);
            let reward_pool = ten_percent.mul_floor(self.accumulative_reward_pool);
            Ok(reward_pool)
        }

        #[ink(message)]
        pub fn pay_node_reward(
            &mut self,
            account_id: AccountId,
            amount: Balance,
        ) -> Result<(), Error> {
            self.only_callable_by(self.node_reward_contract)?;
            let _ = self.env().transfer(account_id, amount);
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_sub(amount);
            Ok(())
        }

        #[ink(message)]
        pub fn deduct_from_reward_pool(&mut self, amount: Balance) -> Result<(), Error> {
            self.only_callable_by(self.node_reward_contract)?;
            self.accumulative_reward_pool = self.accumulative_reward_pool.saturating_sub(amount);
            Ok(())
        }

        // ========== Merchant Operations Messages ==========
        #[ink(message, payable)]
        pub fn process_merchant_payment(&mut self, merchant_id: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.merchant_contract)?;
            let received_amount = self.env().transferred_value();
            self.merchant_volume = self.merchant_volume.saturating_add(received_amount);

            // give merchant votes
            let votes = self.calc_votes_from_d9(received_amount);
            let add_vote_result = self
                .env()
                .extension()
                .add_voting_interests(merchant_id, votes);
            if add_vote_result.is_err() {
                return Err(Error::AddingVotes);
            }
            Ok(())
        }

        #[ink(message)]
        pub fn merchant_user_redeem_d9(
            &mut self,
            user_account: AccountId,
            redeemable_usdt: Balance,
        ) -> Result<Balance, Error> {
            self.only_callable_by(self.merchant_contract)?;

            if redeemable_usdt == 0 {
                return Err(Error::RedeemableUSDTZero);
            }

            // Get current D9 amount for the user's USDT
            let current_d9_amount =
                self.get_exchange_amount(Direction(Currency::Usdt, Currency::D9), redeemable_usdt)?;

            // Calculate current rate (D9 per USDT)
            // Using integer math to avoid decimals
            let current_rate = current_d9_amount
                .saturating_mul(Self::PRICE_PRECISION)
                .saturating_div(redeemable_usdt);

            // Get stored highest rate
            let mut highest_rate = self.get_highest_price();

            if highest_rate == 0 {
                // First time - initialize with current rate
                highest_rate = current_rate;
                self.set_highest_price(highest_rate);
            }

            // Update highest rate if current is better
            if current_rate > highest_rate {
                highest_rate = current_rate;
                self.set_highest_price(highest_rate);
            }

            // Calculate minimum acceptable rate (70% of highest)
            let min_acceptable_rate = highest_rate
                .saturating_mul(Self::PERCENT_PROTECT)
                .saturating_div(100);

            // Use the better rate
            let effective_rate = if current_rate >= min_acceptable_rate {
                current_rate // Current rate is acceptable
            } else {
                min_acceptable_rate // Use protected rate
            };

            // Calculate final D9 amount using effective rate
            let final_d9_amount = redeemable_usdt
                .saturating_mul(effective_rate)
                .saturating_div(Self::PRICE_PRECISION);

            self.env()
                .transfer(user_account, final_d9_amount)
                .map_err(|_| Error::FailedToTransferD9ToUser)?;
            Ok(final_d9_amount)
        }

        // ========== Admin Operations Messages ==========
        #[ink(message)]
        pub fn change_merchant_contract(
            &mut self,
            merchant_contract: AccountId,
        ) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.merchant_contract = merchant_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_node_reward_contract(
            &mut self,
            node_reward_contract: AccountId,
        ) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.node_reward_contract = node_reward_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_amm_contract(&mut self, amm_contract: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.amm_contract = amm_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn change_main_contract(&mut self, main_contract: AccountId) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            self.main_contract = main_contract;
            Ok(())
        }

        #[ink(message)]
        pub fn send_to(&mut self, to: AccountId, amount: Balance) -> Result<(), Error> {
            self.only_callable_by(self.admin)?;
            let _ = self.env().transfer(to, amount);
            Ok(())
        }

        #[ink(message)]
        pub fn set_code(&mut self, code_hash: [u8; 32]) {
            let caller = self.env().caller();
            assert!(caller == self.admin, "Only admin can set code hash.");
            ink::env::set_code_hash(&code_hash).unwrap_or_else(|err| {
                panic!(
                    "Failed to `set_code_hash` to {:?} due to {:?}",
                    code_hash, err
                )
            });
            ink::env::debug_println!("Switched code hash to {:?}.", code_hash);
        }

        // ========== Helper Functions ==========
        // Price management helpers
        fn get_highest_price(&self) -> Balance {
            self.volume_at_index
                .get(Self::PRICE_STORAGE_KEY)
                .unwrap_or(0)
        }

        fn set_highest_price(&mut self, price: Balance) {
            self.volume_at_index.insert(Self::PRICE_STORAGE_KEY, &price);
        }

        // Session calculation helpers
        fn calculate_session_delta(
            &self,
            session_index: u32,
            current_volume: Balance,
        ) -> Result<Balance, Error> {
            let previous_index = self.get_previous_valid_session_index(session_index);
            let previous_volume = self.volume_at_index.get(previous_index).unwrap_or(0);
            let session_delta = current_volume.saturating_sub(previous_volume);
            Ok(session_delta)
        }

        fn get_previous_valid_session_index(&self, last_session: u32) -> u32 {
            let mut previous_index = last_session.saturating_sub(1);
            while previous_index > 0 && self.volume_at_index.get(previous_index).is_none() {
                previous_index = previous_index.saturating_sub(1);
            }
            previous_index
        }

        // Voting calculation helper
        fn calc_votes_from_d9(&self, d9_amount: Balance) -> u64 {
            let one_d9: Balance = 1_000_000_000_000;
            let votes = d9_amount.saturating_div(one_d9);
            votes as u64
        }

        // External contract interaction helpers
        fn get_exchange_amount(
            &self,
            direction: Direction,
            amount: Balance,
        ) -> Result<Balance, Error> {
            build_call::<D9Environment>()
                .call(self.amm_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("calculate_exchange")))
                        .push_arg(direction)
                        .push_arg(amount),
                )
                .returns::<Result<Balance, Error>>()
                .invoke()
        }

        fn get_total_burned(&self) -> Balance {
            build_call::<D9Environment>()
                .call(self.main_contract)
                .gas_limit(0)
                .exec_input(ExecutionInput::new(Selector::new(selector_bytes!(
                    "get_total_burned"
                ))))
                .returns::<Balance>()
                .invoke()
        }

        // Access control helper
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
        use ink::env::test::DefaultAccounts;
        use ink::env::{test, DefaultEnvironment};

        // Test helper functions
        fn default_accounts() -> DefaultAccounts<DefaultEnvironment> {
            test::default_accounts::<DefaultEnvironment>()
        }

        fn set_caller(caller: AccountId) {
            test::set_caller::<DefaultEnvironment>(caller);
        }

        fn set_value_transferred(value: Balance) {
            test::set_value_transferred::<DefaultEnvironment>(value);
        }

        fn get_account_balance(account: AccountId) -> Balance {
            test::get_account_balance::<DefaultEnvironment>(account).unwrap_or(0)
        }

        fn set_account_balance(account: AccountId, balance: Balance) {
            test::set_account_balance::<DefaultEnvironment>(account, balance);
        }

        fn _advance_block() {
            test::advance_block::<DefaultEnvironment>();
        }

        // Mock contract addresses
        fn mock_main_contract() -> AccountId {
            AccountId::from([0x01; 32])
        }

        fn mock_merchant_contract() -> AccountId {
            AccountId::from([0x02; 32])
        }

        fn mock_node_reward_contract() -> AccountId {
            AccountId::from([0x03; 32])
        }

        fn mock_amm_contract() -> AccountId {
            AccountId::from([0x04; 32])
        }

        fn create_default_rewards_aggregator() -> RewardsAggregator {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            )
        }

        // Constructor tests
        #[ink::test]
        fn test_constructor_initializes_correctly() {
            let accounts = default_accounts();
            set_caller(accounts.alice);

            let pool = create_default_rewards_aggregator();

            assert_eq!(pool.admin, accounts.alice);
            assert_eq!(pool.main_contract, mock_main_contract());
            assert_eq!(pool.merchant_contract, mock_merchant_contract());
            assert_eq!(pool.node_reward_contract, mock_node_reward_contract());
            assert_eq!(pool.amm_contract, mock_amm_contract());
            assert_eq!(pool.merchant_volume, 0);
            assert_eq!(pool.last_session, 0);
            assert_eq!(pool.accumulative_reward_pool, 0);
        }

        // Getter function tests
        #[ink::test]
        fn test_get_accumulative_reward_pool() {
            let mut pool = create_default_rewards_aggregator();

            assert_eq!(pool.get_accumulative_reward_pool(), 0);

            pool.accumulative_reward_pool = 1000;
            assert_eq!(pool.get_accumulative_reward_pool(), 1000);
        }

        #[ink::test]
        fn test_get_merchant_volume() {
            let mut pool = create_default_rewards_aggregator();

            assert_eq!(pool.get_merchant_volume(), 0);

            pool.merchant_volume = 5000;
            assert_eq!(pool.get_merchant_volume(), 5000);
        }

        #[ink::test]
        fn test_get_session_volume() {
            let mut pool = create_default_rewards_aggregator();

            // Non-existing session should return 0
            assert_eq!(pool.get_session_volume(1), 0);

            // Add volume for session 1
            pool.volume_at_index.insert(1, &1000);
            assert_eq!(pool.get_session_volume(1), 1000);

            // Add volume for session 2
            pool.volume_at_index.insert(2, &2500);
            assert_eq!(pool.get_session_volume(2), 2500);
        }

        #[ink::test]
        fn test_get_price_protection_info() {
            let mut pool = create_default_rewards_aggregator();

            // Initially should return (0, 0)
            let (highest, min_protected) = pool.get_price_protection_info();
            assert_eq!(highest, 0);
            assert_eq!(min_protected, 0);

            // Set highest price
            pool.set_highest_price(1000);
            let (highest, min_protected) = pool.get_price_protection_info();
            assert_eq!(highest, 1000);
            assert_eq!(min_protected, 700); // 70% of 1000

            // Test with larger value
            pool.set_highest_price(10000);
            let (highest, min_protected) = pool.get_price_protection_info();
            assert_eq!(highest, 10000);
            assert_eq!(min_protected, 7000); // 70% of 10000
        }

        // Helper function tests
        #[ink::test]
        fn test_price_storage_and_retrieval() {
            let mut pool = create_default_rewards_aggregator();

            // Initially should be 0
            assert_eq!(pool.get_highest_price(), 0);

            // Set and retrieve price
            pool.set_highest_price(1500);
            assert_eq!(pool.get_highest_price(), 1500);

            // Update price
            pool.set_highest_price(2000);
            assert_eq!(pool.get_highest_price(), 2000);
        }

        #[ink::test]
        fn test_get_previous_valid_session_index() {
            let mut pool = create_default_rewards_aggregator();

            // With no sessions, should return 0
            assert_eq!(pool.get_previous_valid_session_index(5), 0);

            // Add some sessions
            pool.volume_at_index.insert(1, &100);
            pool.volume_at_index.insert(3, &300);
            pool.volume_at_index.insert(5, &500);

            // Previous of 5 should be 3
            assert_eq!(pool.get_previous_valid_session_index(5), 3);

            // Previous of 3 should be 1
            assert_eq!(pool.get_previous_valid_session_index(3), 1);

            // Previous of 2 should be 1
            assert_eq!(pool.get_previous_valid_session_index(2), 1);

            // Previous of 1 should be 0
            assert_eq!(pool.get_previous_valid_session_index(1), 0);
        }

        #[ink::test]
        fn test_calc_votes_from_d9() {
            let pool = create_default_rewards_aggregator();

            // 1 D9 = 1 vote
            let one_d9: Balance = 1_000_000_000_000;
            assert_eq!(pool.calc_votes_from_d9(one_d9), 1);

            // 10 D9 = 10 votes
            assert_eq!(pool.calc_votes_from_d9(one_d9 * 10), 10);

            // 0.5 D9 = 0 votes (truncated)
            assert_eq!(pool.calc_votes_from_d9(one_d9 / 2), 0);

            // 1.9 D9 = 1 vote (truncated)
            assert_eq!(pool.calc_votes_from_d9(one_d9 * 19 / 10), 1);
        }

        #[ink::test]
        fn test_calculate_session_delta() {
            let mut pool = create_default_rewards_aggregator();

            // First session should have delta equal to total volume
            let delta = pool.calculate_session_delta(1, 1000).unwrap();
            assert_eq!(delta, 1000);

            // Add volume for session 1
            pool.volume_at_index.insert(1, &1000);

            // Session 2 with volume 1500 should have delta of 500
            let delta = pool.calculate_session_delta(2, 1500).unwrap();
            assert_eq!(delta, 500);

            // Add volume for session 2
            pool.volume_at_index.insert(2, &1500);

            // Session 3 with same volume should have delta of 0
            let delta = pool.calculate_session_delta(3, 1500).unwrap();
            assert_eq!(delta, 0);
        }

        // Access control tests
        #[ink::test]
        fn test_only_callable_by() {
            let accounts = default_accounts();
            let pool = create_default_rewards_aggregator();

            // Should succeed when caller matches
            set_caller(accounts.alice);
            assert!(pool.only_callable_by(accounts.alice).is_ok());

            // Should fail when caller doesn't match
            set_caller(accounts.bob);
            match pool.only_callable_by(accounts.alice) {
                Err(Error::OnlyCallableBy(expected)) => {
                    assert_eq!(expected, accounts.alice);
                }
                _ => panic!("Expected OnlyCallableBy error"),
            }
        }

        // Admin operation tests
        #[ink::test]
        fn test_change_merchant_contract() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            let mut pool = RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            );
            let new_merchant = AccountId::from([0x05; 32]);

            // Should fail if not admin
            set_caller(accounts.bob);
            assert!(pool.change_merchant_contract(new_merchant).is_err());

            // Should succeed if admin
            set_caller(accounts.alice);
            assert!(pool.change_merchant_contract(new_merchant).is_ok());
            assert_eq!(pool.merchant_contract, new_merchant);
        }

        #[ink::test]
        fn test_change_node_reward_contract() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            let mut pool = RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            );
            let new_node_reward = AccountId::from([0x06; 32]);

            // Should fail if not admin
            set_caller(accounts.bob);
            assert!(pool.change_node_reward_contract(new_node_reward).is_err());

            // Should succeed if admin
            set_caller(accounts.alice);
            assert!(pool.change_node_reward_contract(new_node_reward).is_ok());
            assert_eq!(pool.node_reward_contract, new_node_reward);
        }

        #[ink::test]
        fn test_change_amm_contract() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            let mut pool = RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            );
            let new_amm = AccountId::from([0x07; 32]);

            // Should fail if not admin
            set_caller(accounts.bob);
            assert!(pool.change_amm_contract(new_amm).is_err());

            // Should succeed if admin
            set_caller(accounts.alice);
            assert!(pool.change_amm_contract(new_amm).is_ok());
            assert_eq!(pool.amm_contract, new_amm);
        }

        #[ink::test]
        fn test_change_main_contract() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            let mut pool = RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            );
            let new_main = AccountId::from([0x08; 32]);

            // Should fail if not admin
            set_caller(accounts.bob);
            assert!(pool.change_main_contract(new_main).is_err());

            // Should succeed if admin
            set_caller(accounts.alice);
            assert!(pool.change_main_contract(new_main).is_ok());
            assert_eq!(pool.main_contract, new_main);
        }

        #[ink::test]
        fn test_send_to() {
            let accounts = default_accounts();
            set_caller(accounts.alice);
            let mut pool = RewardsAggregator::new(
                mock_main_contract(),
                mock_merchant_contract(),
                mock_node_reward_contract(),
                mock_amm_contract(),
            );

            // Set up contract balance
            let contract_addr = test::callee::<DefaultEnvironment>();
            set_account_balance(contract_addr, 10000);

            // Should fail if not admin
            set_caller(accounts.bob);
            assert!(pool.send_to(accounts.charlie, 1000).is_err());

            // Should succeed if admin
            set_caller(accounts.alice);
            let _initial_balance = get_account_balance(accounts.charlie);
            assert!(pool.send_to(accounts.charlie, 1000).is_ok());

            // Note: In unit tests, transfers don't actually move funds
            // In a real environment, we would check that charlie's balance increased
        }

        // Pool operation tests
        #[ink::test]
        fn test_deduct_from_reward_pool() {
            let mut pool = create_default_rewards_aggregator();
            pool.accumulative_reward_pool = 5000;

            // Should fail if not node reward contract
            set_caller(mock_merchant_contract());
            assert!(pool.deduct_from_reward_pool(1000).is_err());

            // Should succeed if node reward contract
            set_caller(mock_node_reward_contract());
            assert!(pool.deduct_from_reward_pool(1000).is_ok());
            assert_eq!(pool.accumulative_reward_pool, 4000);

            // Test underflow protection
            assert!(pool.deduct_from_reward_pool(5000).is_ok());
            assert_eq!(pool.accumulative_reward_pool, 0); // Should saturate at 0
        }

        #[ink::test]
        fn test_pay_node_reward() {
            let accounts = default_accounts();
            let mut pool = create_default_rewards_aggregator();
            pool.accumulative_reward_pool = 5000;

            // Set up contract balance
            let contract_addr = test::callee::<DefaultEnvironment>();
            set_account_balance(contract_addr, 10000);

            // Should fail if not node reward contract
            set_caller(mock_merchant_contract());
            assert!(pool.pay_node_reward(accounts.bob, 1000).is_err());

            // Should succeed if node reward contract
            set_caller(mock_node_reward_contract());
            assert!(pool.pay_node_reward(accounts.bob, 1000).is_ok());
            assert_eq!(pool.accumulative_reward_pool, 4000);
        }

        // More complex tests would require mocking chain extension responses
        // These would be better suited for integration tests

        // Update pool and retrieve tests
        #[ink::test]
        fn test_update_pool_and_retrieve_access_control() {
            let mut pool = create_default_rewards_aggregator();

            // Should fail if not node reward contract
            set_caller(mock_merchant_contract());
            assert!(pool.update_pool_and_retrieve(1).is_err());
        }

        // Note: Full update_pool_and_retrieve tests would require mocking get_total_volume

        // Merchant payment tests
        #[ink::test]
        fn test_process_merchant_payment_access_control() {
            let accounts = default_accounts();
            let mut pool = create_default_rewards_aggregator();

            // Should fail if not merchant contract
            set_caller(accounts.alice);
            set_value_transferred(1000);
            assert!(pool.process_merchant_payment(accounts.bob).is_err());
        }

        // Note: Full process_merchant_payment tests would require mocking chain extension

        // Merchant user redeem tests
        #[ink::test]
        fn test_merchant_user_redeem_d9_access_control() {
            let accounts = default_accounts();
            let mut pool = create_default_rewards_aggregator();

            // Should fail if not merchant contract
            set_caller(accounts.alice);
            assert!(pool.merchant_user_redeem_d9(accounts.bob, 100).is_err());
        }

        #[ink::test]
        fn test_merchant_user_redeem_d9_zero_amount() {
            let accounts = default_accounts();
            let mut pool = create_default_rewards_aggregator();

            // Should fail with zero amount
            set_caller(mock_merchant_contract());
            match pool.merchant_user_redeem_d9(accounts.bob, 0) {
                Err(Error::RedeemableUSDTZero) => {}
                _ => panic!("Expected RedeemableUSDTZero error"),
            }
        }

        // Note: Full merchant_user_redeem_d9 tests would require mocking get_exchange_amount

        // Edge case tests
        #[ink::test]
        fn test_arithmetic_overflow_protection() {
            let mut pool = create_default_rewards_aggregator();

            // Test merchant volume overflow protection
            pool.merchant_volume = Balance::MAX - 100;
            set_caller(mock_merchant_contract());
            set_value_transferred(200);

            // This should use saturating_add and not panic
            // Note: This test would need chain extension mocking to fully execute

            // Test accumulative reward pool overflow
            pool.accumulative_reward_pool = Balance::MAX - 100;
            let result = pool.accumulative_reward_pool.saturating_add(200);
            assert_eq!(result, Balance::MAX);
        }

        #[ink::test]
        fn test_session_edge_cases() {
            let mut pool = create_default_rewards_aggregator();

            // Test with session 0
            assert_eq!(pool.get_previous_valid_session_index(0), 0);

            // Test with large session numbers
            pool.volume_at_index.insert(u32::MAX - 1, &1000);
            assert_eq!(pool.get_session_volume(u32::MAX - 1), 1000);
        }

        #[ink::test]
        fn test_price_precision_constants() {
            // Verify constants are as expected
            assert_eq!(RewardsAggregator::PRICE_STORAGE_KEY, 999_999_999);
            assert_eq!(RewardsAggregator::PRICE_PRECISION, 1_000_000);
            assert_eq!(RewardsAggregator::PERCENT_PROTECT, 70);
        }

        // Integration-style test scenarios (would need mocking)
        #[ink::test]
        fn test_session_progression_scenario() {
            let mut pool = create_default_rewards_aggregator();

            // Simulate multiple sessions
            pool.volume_at_index.insert(1, &1000);
            pool.volume_at_index.insert(2, &2000);
            pool.volume_at_index.insert(3, &3500);

            // Test delta calculations
            assert_eq!(pool.calculate_session_delta(4, 5000).unwrap(), 1500);

            // Add volume for session 4
            pool.volume_at_index.insert(4, &5000);

            // Session 5 with same volume should have delta of 0
            assert_eq!(pool.calculate_session_delta(5, 5000).unwrap(), 0);
        }

        #[ink::test]
        fn test_price_protection_scenario() {
            let mut pool = create_default_rewards_aggregator();

            // Simulate price history
            pool.set_highest_price(2_000_000); // 2 D9/USDT with precision

            // Current rate calculation would use 70% protection
            let (highest, protected) = pool.get_price_protection_info();
            assert_eq!(highest, 2_000_000);
            assert_eq!(protected, 1_400_000); // 70% of highest
        }
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
            let constructor = RewardsAggregatorRef::default();

            // When
            let contract_account_id = client
                .instantiate(
                    "rewards_aggregator",
                    &ink_e2e::alice(),
                    constructor,
                    0,
                    None,
                )
                .await
                .expect("instantiate failed")
                .account_id;

            // Then
            let get = build_message::<RewardsAggregatorRef>(contract_account_id.clone())
                .call(|rewards_aggregator| rewards_aggregator.get());
            let get_result = client.call_dry_run(&ink_e2e::alice(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            Ok(())
        }

        /// We test that we can read and write a value from the on-chain contract contract.
        #[ink_e2e::test]
        async fn it_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = RewardsAggregatorRef::new(false);
            let contract_account_id = client
                .instantiate("rewards_aggregator", &ink_e2e::bob(), constructor, 0, None)
                .await
                .expect("instantiate failed")
                .account_id;

            let get = build_message::<RewardsAggregatorRef>(contract_account_id.clone())
                .call(|rewards_aggregator| rewards_aggregator.get());
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            // When
            let flip = build_message::<RewardsAggregatorRef>(contract_account_id.clone())
                .call(|rewards_aggregator| rewards_aggregator.flip());
            let _flip_result = client
                .call(&ink_e2e::bob(), flip, 0, None)
                .await
                .expect("flip failed");

            // Then
            let get = build_message::<RewardsAggregatorRef>(contract_account_id.clone())
                .call(|rewards_aggregator| rewards_aggregator.get());
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), true));

            Ok(())
        }
    }
}
