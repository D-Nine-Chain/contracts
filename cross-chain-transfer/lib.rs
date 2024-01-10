#![cfg_attr(not(feature = "std"), no_std, no_main)]
pub use d9_chain_extension::D9Environment;
#[ink::contract(env = D9Environment)]
mod cross_chain_transfer {
    use super::*;

    use scale::{ Decode, Encode };
    use ink::storage::Mapping;
    use ink::selector_bytes;
    use ink::env::{
        call::{ build_call, ExecutionInput, Selector },
        hash_encoded,
        hash::{ Keccak256, HashOutput },
    };
    use ink::prelude::string::String;
    #[ink(storage)]
    pub struct CrossChainTransfer {
        //user transaction nonce
        user_transaction_nonce: Mapping<AccountId, u64>,
        admin: AccountId,
        new_admin: AccountId,
        controller: AccountId,
        usdt_contract: AccountId,
        transactions: Mapping<String, Transaction>,
    }

    #[ink(event)]
    pub struct CommitCreated {
        #[ink(topic)]
        pub transaction_id: String,
        #[ink(topic)]
        pub from_address: AccountId,
        #[ink(topic)]
        pub amount: u128,
    }

    #[ink(event)]
    pub struct DispatchCompleted {
        #[ink(topic)]
        pub tx_id: String,
        #[ink(topic)]
        pub to_address: AccountId,
        #[ink(topic)]
        pub amount: u128,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub enum Chain {
        D9,
        TRON,
    }

    #[derive(scale::Encode, scale::Decode, Clone, PartialEq, Eq, Copy, Debug)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub enum AddressType {
        Tron([u8; 21]),
        D9(AccountId),
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub struct Transaction {
        transaction_id: String,
        transaction_type: TransactionType,
        from_chain: Chain,
        from_address: AddressType,
        to_address: AddressType,
        amount: u128,
        timestamp: Timestamp,
    }
    // note how do i manage from_address and to to_address for the different chains?

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout))]
    pub enum TransactionType {
        Commit,
        Dispatch,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        Restrictedto(AccountId),
        AmountMustBeGreaterThanZero,
        TransactionAlreadyExists,
        InvalidAddressLength(Chain),
        InvalidHexString,
        DecodedHexLengthInvalid,
        TronAddressInvalidByteLength,
        InvalidTronAddress,
        TronDecodeError,
        UnableToSendUSDT,
        InsufficientAllowance,
        UserUSDTBalanceInsufficient,
        D9orUSDTProvidedLiquidityAtZero,
    }

    impl CrossChainTransfer {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(usdt_contract: AccountId) -> Self {
            Self {
                user_transaction_nonce: Mapping::new(),
                admin: Self::env().caller(),
                new_admin: AccountId::from([0u8; 32]),
                controller: Self::env().caller(),
                usdt_contract,
                transactions: Mapping::new(),
            }
        }

        #[ink(message)]
        pub fn generate_tx_id(&self, user_id: AccountId) -> String {
            self.create_hash(user_id, self.get_current_nonce(user_id))
        }

        /// get last transaction. function is called on both chains.
        #[ink(message)]
        pub fn get_last_transaction(&self, user_id: AccountId) -> Option<Transaction> {
            let last_nonce = self.get_current_nonce(user_id).saturating_sub(1);
            let tx_id = self.create_hash(user_id, last_nonce);
            self.transactions.get(&tx_id)
        }

        /// Helper function to get the current transaction nonce for a user
        #[ink(message)]
        pub fn get_current_nonce(&self, user_id: AccountId) -> u64 {
            self.user_transaction_nonce.get(user_id).unwrap_or_default()
        }

        /// Common logic to create a hash from a user ID and nonce
        fn create_hash(&self, user_id: AccountId, nonce: u64) -> String {
            let encodable = (nonce, user_id);
            let mut output = <Keccak256 as HashOutput>::Type::default();
            hash_encoded::<Keccak256, _>(&encodable, &mut output);
            hex::encode(output)
        }
        #[ink(message)]
        pub fn get_transaction(&self, tx_id: String) -> Option<Transaction> {
            self.transactions.get(&tx_id)
        }
        #[ink(message)]
        pub fn asset_commit(
            &mut self,
            transaction_id: String,
            from_address: AccountId,
            to_address: [u8; 21],
            amount: Balance
        ) -> Result<String, Error> {
            // only controller
            let caller_check = self.only_callable_by(self.controller);
            if let Err(e) = caller_check {
                return Err(e);
            }

            if to_address.len() != 21 {
                return Err(Error::TronAddressInvalidByteLength);
            }
            //validate commit
            let validate_commit_result = self.validate_commit(&to_address, amount);

            if let Err(e) = validate_commit_result {
                return Err(e);
            }

            //prepare transaction execution
            let unique_transaction_check = self.ensure_unique_transaction(&transaction_id);
            if let Err(e) = unique_transaction_check {
                return Err(e);
            }

            // validate usdt
            let vaidate_usdt_transfer_result = self.validate_usdt_transfer(from_address, amount);
            if let Err(e) = vaidate_usdt_transfer_result {
                return Err(e);
            }

            //receive usdt
            let receive_usdt_result = self.receive_usdt(from_address, amount);
            if let Err(e) = receive_usdt_result {
                return Err(e);
            }

            //store transaction
            let transaction = Transaction {
                transaction_id: transaction_id.clone(),
                transaction_type: TransactionType::Commit,
                from_chain: Chain::D9,
                from_address: AddressType::D9(from_address),
                to_address: AddressType::Tron(to_address),
                amount,
                timestamp: self.env().block_timestamp(),
            };

            self.increase_transaction_nonce(from_address);
            self.transactions.insert(transaction_id.clone(), &transaction);

            self.env().emit_event(CommitCreated {
                transaction_id: transaction_id.clone(),
                from_address,
                amount,
            });
            Ok(transaction_id)
        }

        #[ink(message)]
        pub fn asset_dispatch(
            &mut self,
            from_address: [u8; 21],
            to_address: AccountId,
            amount: Balance
        ) -> Result<String, Error> {
            let caller_check = self.only_callable_by(self.controller);
            if let Err(e) = caller_check {
                return Err(e);
            }

            let tx_id = self.generate_tx_id(to_address);
            let unique_transaction_check = self.ensure_unique_transaction(&tx_id);
            if let Err(e) = unique_transaction_check {
                return Err(e);
            }

            let transaction = Transaction {
                transaction_id: tx_id.clone(),
                transaction_type: TransactionType::Dispatch,
                from_chain: Chain::TRON,
                from_address: AddressType::Tron(from_address),
                to_address: AddressType::D9(to_address),
                amount,
                timestamp: self.env().block_timestamp(),
            };
            let send_usdt_result = self.send_usdt(to_address, amount);
            if send_usdt_result.is_err() {
                return Err(Error::UnableToSendUSDT);
            }

            self.transactions.insert(tx_id.clone(), &transaction);
            self.increase_transaction_nonce(to_address);
            self.env().emit_event(DispatchCompleted {
                tx_id: tx_id.clone(),
                to_address,
                amount,
            });
            Ok(tx_id)
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
            self.new_admin = AccountId::from([0u8; 32]);
        }

        #[ink(message)]
        pub fn cancel_admin_transfer(&mut self) {
            assert_eq!(self.admin, self.env().caller());
            self.new_admin = AccountId::from([0u8; 32]);
        }

        fn validate_commit(&self, to_address: &[u8; 21], amount: Balance) -> Result<(), Error> {
            if to_address.len() != 21 {
                return Err(Error::InvalidAddressLength(Chain::TRON));
            }
            if amount == 0 {
                return Err(Error::AmountMustBeGreaterThanZero);
            }
            Ok(())
        }

        fn increase_transaction_nonce(&mut self, user_id: AccountId) {
            let user_transaction_nonce = self.user_transaction_nonce
                .get(&user_id)
                .unwrap_or_default();
            let new_nonce = user_transaction_nonce.saturating_add(1);
            self.user_transaction_nonce.insert(user_id, &new_nonce);
        }

        fn ensure_unique_transaction(&self, tx_id: &String) -> Result<(), Error> {
            if self.transactions.contains(tx_id) {
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

        //   fn hex_to_bytes(&self, hex_str: &str) -> Result<[u8; 21], Error> {
        //       let hex_decode_result = hex::decode(hex_str);
        //       if hex_decode_result.is_err() {
        //           return Err(Error::InvalidHexString);
        //       }
        //       let hex_vec = hex_decode_result.unwrap();
        //       if hex_vec.len() != 21 {
        //           return Err(Error::DecodedHexLengthInvalid);
        //       }
        //       let mut arr = [0u8; 21];
        //       arr.copy_from_slice(&hex_vec);
        //       Ok(arr)
        //   }
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
        use ink::env::test::default_accounts;
        use ink::env::DefaultEnvironment;
        use bs58;
        /// We test a simple use case of our contract.
        #[ink::test]
        fn it_works() {
            let mut cross_chain_transfer = CrossChainTransfer::new(AccountId::from([0x1; 32]));
            let address = cross_chain_transfer.bytes_to_account_id([
                94, 211, 105, 27, 83, 160, 52, 54, 247, 62, 240, 54, 250, 98, 15, 240, 78, 47, 162, 143,
                137, 234, 193, 167, 30, 39, 243, 143, 192, 126, 128, 40,
            ]);

            println!("address: {:?}", hex::encode(address));
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
        use ink_e2e::{ build_message, account_id, AccountKeyring };
        use d9_usdt::d9_usdt::D9USDT;
        use d9_usdt::d9_usdt::D9USDTRef;
        /// The End-to-End test `Result` type.
        type E2EResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

        /// We test that we can upload and instantiate the contract using its default constructor.
        #[ink_e2e::test]
        async fn default_works(mut client: ink_e2e::Client<C, E>) -> E2EResult<()> {
            let initial_supply: Balance = 100_000_000_000_000;
            let usdt_constructor = D9USDTRef::new(initial_supply);
            let usdt_address = client
                .instantiate("d9_usdt", &ink_e2e::alice(), usdt_constructor, 0, None).await
                .expect("failed to instantiate usdt").account_id;

            let grant_allowance = build_message::<D9USDTRef>(contract.clone()).call(|usdt|
                usdt.approve(user, contract, amount)
            );
            let call_result = client.call_dry_run(user, &grant_allowance, 0, None).await;

            let constructor = CrossChainTransferRef::new(usdt_address);
            let contract_account_id = client
                .instantiate("cross_chain_transfer", &ink_e2e::alice(), constructor, 0, None).await
                .expect("instantiate failed").account_id;

            let d9_bytes = [
                94, 211, 105, 27, 83, 160, 52, 54, 247, 62, 240, 54, 250, 98, 15, 240, 78, 47, 162, 143,
                137, 234, 193, 167, 30, 39, 243, 143, 192, 126, 128, 40,
            ];
            let tron_bytes = [
                41, 219, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 254, 117, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            let send_amount = 10000;
            let commit_transaction = build_message::<CrossChainTransferRef>(
                contract_account_id.clone()
            ).call(|cross_chain_transfer|
                cross_chain_transfer.create_commit_transaction(d9_bytes, tron_bytes, send_amount)
            );
            let call_result = client.call_dry_run(
                &ink_e2e::alice(),
                &commit_transaction,
                0,
                None
            ).await;

            Ok(())
        }
    }
}
