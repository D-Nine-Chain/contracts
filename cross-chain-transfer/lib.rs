#![cfg_attr(not(feature = "std"), no_std, no_main)]
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod cross_chain_transfer {
    use super::*;

    use ink::env::hash::Keccak256;
    use ink_e2e::subxt::tx;
    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use ink::selector_bytes;
    use ink::env::{
        call::{ build_call, ExecutionInput, Selector },
        hash_encoded,
        hash::{ Keccak256, HashOutput },
    };
    use ink::prelude::{ string::String, vec };
    #[ink(storage)]
    pub struct CrossChainTransfer {
        //user transaction nonce
        user_transaction_nonce: Mapping<AccountId, u64>,
        admin: AccountId,
        new_admin: AccountId,
        controller: AccountId,
        usdt_contract: AcccountId,
        transactions: Mapping<String, Option<Transaction>>,
    }
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Chain {
        D9,
        TRON,
    }

    #[derive(scale::Encode, scale::Decode, Clone, PartialEq, Eq, Debug, Default)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum AddressType {
        Tron([u8; 20]),
        D9([u8; 32]),
    }

    impl AddressType {
        pub fn from_tron_raw_form(address: [u8; 20]) -> Self {
            AddressType::Tron(address)
        }

        pub fn from_tron_raw(address: [u8; 32]) -> Self {
            AddressType::Substrate(address)
        }

        pub fn from_tron_str(address: &str) -> Result<Self, Error> {
            let str_length = address.len();
            if str_length != 34 || !address.starts_with('T') {
                return Err(Error::InvalidTronAddress);
            }

            let address_without_prefix = &address[1..];
            let decode_result = Decode::new(address_without_prefix.as_bytes())
                .with_check(None)
                .into_vec();
            if decode_result.is_err() {
                return Err(Error::TronDecodeError);
            }
            if bytes.len() != 20 {
                return Err(Error::TronAddressInvalidByteLength);
            }
            let mut address_bytes = [0u8; 20];
            address_bytes.copy_from_slice(&bytes);
            Ok(Self::from_tron_raw_form(address_bytes))
        }
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub struct Transaction {
        transaction_type: TransactionType,
        fromChain: Chain,
        from_address: AddressType,
        to_address: AddressType,
        amount: u128,
        timestamp: Timestamp,
    }
    // note how do i manage from_address and to to_address for the different chains?

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum TransactionType {
        Commit,
        Transfer,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        Restrictedto(AccountId),
        TransactionAlreadyExists,
        UnableToSendUSDT,
        UserUSDTBalanceInsufficient,
        UserUSDTBalanceInsufficient,
        InvalidAddressLength,
        InvalidHexString,
        TronAddressInvalidByteLength,
        InvalidTronAddress,
        TronDecodeError,
    }

    impl CrossChainTransfer {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(usdt_contract: AccountId) -> Self {
            Self {
                user_transaction_nonce: Mapping::new(),
                admin: self.env().caller(),
                new_admin: AccountId::default(),
                controller: self.env().caller(),
                usdt_contract,
                transactions: Mapping::new(),
            }
        }

        #[ink(message)]
        pub fn generate_tx_id(
            &self,
            user_id: AccountId,
            from: AddressType,
            to: AddressType,
            amount: Balance
        ) -> String {
            let user_transaction_none = self.user_transaction_nonce
                .get(&user_id)
                .unwrap_or_default();
            let encodable = (user_transaction_none, user_id);
            let mut output = <Keccak256 as HashOutput>::Type::default();
            hash_encoded(&encodable, &mut output);
            hex::encode(output)
        }

        #[ink(message)]
        pub fn get_last_transaction(&self, user_id: AccountId) -> Option<Transaction> {
            let user_transaction_nonce = self.user_transaction_nonce
                .get(&user_id)
                .unwrap_or_default();
            let transaction_id = self.generate_tx_id(user_id);
        }

        #[ink(message)]
        pub fn create_commit_transaction(
            &mut self,
            from_address: String,
            to_address: String,
            amount: u128
        ) -> Result<String, Error> {
            let caller_check = self.only_callable_by(self.controller);
            if let Err(e) = caller_check {
                return Err(e);
            }

            let user_id = self.env().caller();
            let tx_id = self.generate_tx_id(user_id);
            let unique_transaction_check = self.ensure_unique_transaction(tx_id);
            if let Err(e) = unique_transaction_check {
                return Err(e);
            }

            let vaidate_usdt_transfer_result = self.validate_usdt_transfer(user_id, amount);
            if let Err(e) = vaidate_usdt_transfer_result {
                return Err(e);
            }
            let receive_usdt_result = self.receive_usdt(user_id, amount);
            if let Err(e) = receive_usdt_result {
                return Err(e);
            }

            let transaction = Transaction {
                transaction_type: TransactionType::Commit,
                fromChain: Chain::D9,
                from_address,
                to_address,
                amount,
                timestamp: self.env().block_timestamp(),
            };

            self.increase_transaction_nonce(user_id);
            self.transaction.insert(tx_id.clone(), &transaction);

            Ok(tx_id)
        }

        #[ink(message)]
        pub fn create_transfer_transaction(
            &mut self,
            from_address: String,
            to_address: String,
            amount: u128
        ) -> Result<String, Error> {
            let caller_check = self.only_callable_by(self.controller);
            if let Err(e) = caller_check {
                return Err(e);
            }
            let user_id = self.convert_string_to_accountid(&to_address);
            let tx_id = self.generate_tx_id(user_id, from_address, to_address, amount);

            let unique_transaction_check = self.ensure_unique_transaction(tx_id);
            if let Err(e) = unique_transaction_check {
                return Err(e);
            }

            let transaction = Transaction {
                transaction_type: TransactionType::Transfer,
                fromChain: Chain::TRON,
                from_address,
                to_address,
                amount,
                timestamp: self.env().block_timestamp(),
            };
            let send_usdt_result = self.send_usdt(to_address, amount);
            if send_usdt_result.is_err() {
                return Err(Error::UnableToSendUSDT);
            }

            self.transaction.insert(tx_id.clone(), &transaction);
            self.increase_transaction_nonce(user_id);

            Ok(tx_id)
        }

        fn convert_string_to_accountid(&self, account_str: &str) -> AccountId {
            let mut data = bs58::decode(account_str).into_vec().unwrap();
            let cut_address_vec: Vec<_> = data.drain(1..33).collect();
            let mut array = [0; 32];
            let bytes = &cut_address_vec[..array.len()];
            array.copy_from_slice(bytes);
            let accountId: AccountId = array.into();
            accountId
        }

        fn increase_transaction_nonce(&mut self, user_id: AccountId) {
            let user_transaction_nonce = self.user_transaction_nonce
                .get(&user_id)
                .unwrap_or_default();
            let new_nonce = user_transaction_nonce.saturating_add(1);
            self.user_transaction_nonce.insert(user_id, &new_nonce);
        }

        fn ensure_unique_transaction(&self, tx_id: [u8; 32]) -> Result<(), Error> {
            if self.transaction.contains(&tx_id) {
                return Err(Error::TransactionAlreadyExists);
            }
            Ok(())
        }

        pub fn receive_usdt(&self, sender: AccountId, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer_from")))
                        .push_arg(sender)
                        .push_arg(self.env().account_id())
                        .push_arg(amount)
                        .push_arg([0u8])
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        pub fn send_usdt(&self, recipient: AccountId, amount: Balance) -> Result<(), Error> {
            build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::transfer")))
                        .push_arg(recipient)
                        .push_arg(amount)
                        .push_arg([0u8])
                )
                .returns::<Result<(), Error>>()
                .invoke()
        }

        fn validate_usdt_transfer(&self, account: AccountId, amount: Balance) -> Result<(), Error> {
            let check_balance_result = self.validate_usdt_balance(account, amount);
            if check_balance_result.is_err() {
                return Err(Error::UserUSDTBalanceInsufficient);
            }
            let check_allowance_result = self.validate_usdt_allowance(account, amount);
            if let Err(e) = check_allowance_result {
                return Err(e);
            }
            Ok(())
        }

        fn validate_usdt_balance(
            &self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let usdt_balance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(
                        Selector::new(selector_bytes!("PSP22::balance_of"))
                    ).push_arg(account_id)
                )
                .returns::<Balance>()
                .invoke();
            if usdt_balance < amount {
                return Err(Error::UserUSDTBalanceInsufficient);
            }
            Ok(())
        }

        pub fn validate_usdt_allowance(
            &self,
            owner: AccountId,
            amount: Balance
        ) -> Result<(), Error> {
            let allowance = build_call::<D9Environment>()
                .call(self.usdt_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(selector_bytes!("PSP22::allowance")))
                        .push_arg(owner)
                        .push_arg(self.env().account_id())
                )
                .returns::<Balance>()
                .invoke();
            if allowance < amount {
                return Err(Error::InsufficientAllowance);
            }
            Ok(())
        }

        #[ink(message)]
        pub fn change_controller(&mut self, new_controller: AccountId) {
            assert_eq!(self.admin, self.env().caller());
            self.controller = new_controller;
        }

        #[ink(message)]
        pub fn relinquish_admin(&mut self, new_admin: AccountId) {
            assert_eq!(self.admin, self.env().caller());
            self.new_admin = new_admin;
        }

        #[ink(message)]
        pub fn claim_admin(&mut self) {
            assert_eq!(self.new_admin, self.env().caller());
            self.admin = self.new_admin;
            self.new_admin = AccountId::default();
        }

        #[ink(message)]
        pub fn cancel_admin_transfer(&mut self) {
            assert_eq!(self.admin, self.env().caller());
            self.new_admin = AccountId::default();
        }

        /// restrict the function to be called by `restricted_caller`
        fn only_callable_by(&self, restricted_caller: AccountId) -> Result<(), Error> {
            if self.env().caller() != restricted_caller {
                return Err(Error::Restrictedto(restricted_caller));
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

        /// We test a simple use case of our contract.
        #[ink::test]
        fn it_works() {
            let mut cross_chain_transfer = CrossChainTransfer::new();
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
            let constructor = CrossChainTransferRef::default();

            // When
            let contract_account_id = client
                .instantiate("cross_chain_transfer", &ink_e2e::alice(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            // Then
            let get = build_message::<CrossChainTransferRef>(contract_account_id.clone()).call(
                |cross_chain_transfer| cross_chain_transfer.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::alice(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            Ok(())
        }

        /// We test that we can read and write a value from the on-chain contract contract.
        #[ink_e2e::test]
        async fn it_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            // Given
            let constructor = CrossChainTransferRef::new(false);
            let contract_account_id = client
                .instantiate("cross_chain_transfer", &ink_e2e::bob(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            let get = build_message::<CrossChainTransferRef>(contract_account_id.clone()).call(
                |cross_chain_transfer| cross_chain_transfer.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), false));

            // When
            let flip = build_message::<CrossChainTransferRef>(contract_account_id.clone()).call(
                |cross_chain_transfer| cross_chain_transfer.flip()
            );
            let _flip_result = client
                .call(&ink_e2e::bob(), flip, 0, None).await
                .expect("flip failed");

            // Then
            let get = build_message::<CrossChainTransferRef>(contract_account_id.clone()).call(
                |cross_chain_transfer| cross_chain_transfer.get()
            );
            let get_result = client.call_dry_run(&ink_e2e::bob(), &get, 0, None).await;
            assert!(matches!(get_result.return_value(), true));

            Ok(())
        }
    }
}
