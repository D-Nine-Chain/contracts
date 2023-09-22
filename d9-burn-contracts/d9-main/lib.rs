#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract(env = D9Environment)]
mod d9_main {
    use ink::storage::Mapping;
    use scale::{ Decode, Encode };
    use d9_burn_common::{ D9Environment, Account };
    use ink::env::call::{ build_call, ExecutionInput, Selector };

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9Main {
        admin: AccountId,
        burn_contracts: Vec<AccountId>,
        /// mapping of accountId and code_hash of logic contract to respective account data
        portfolios: Mapping<AccountId, BurnPortfolio>,
    }

    impl D9Main {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(admin: AccountId, burn_contracts: Vec<AccountId>) -> Self {
            Self {
                admin,
                burn_contracts,
                accounts: Default::default(),
            }
        }

        /// A message that can be called on instantiated contracts.
        /// This one flips the value of the stored `bool` from `true`
        /// to `false` and vice versa.
        #[ink(message, payable)]
        pub fn burn(&mut self, burn_contract: AccountId) -> Result<Account, Error> {
            let burn_amount: Balance = self.env().transferred_value();
            let account_id: AccountId = self.env().caller();
            if burn_amount == 0 {
                return Err(Error::BurnAmountIsZero);
            }
            if !self.burn_contracts.contains(&burn_contract) {
                return Err(Error::InvalidBurnContract);
            }
            let burn_result = build_call::<D9Environment>()
                .call(burn_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!("portfolio_execute")))
                        .push_arg(account_id)
                        .push_arg(burn_amount)
                )
                .returns::<Result<Account, Error>>()
                .invoke();
            match burn_result {
                Ok(account) => {}
                Err(e) => {}
            }
        }

        fn _update_account(&mut self, account: Account);
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
            let d9referral = D9BurnMining::default();
            assert_eq!(d9referral.get(), false);
        }

        /// We test a simple use case of our contract.
        #[ink::test]
        fn it_works() {
            let mut d9referral = D9BurnMining::new(false);
            assert_eq!(d9referral.get(), false);
            d9referral.flip();
            assert_eq!(d9referral.get(), true);
        }
    }
}
