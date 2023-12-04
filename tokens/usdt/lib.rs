#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[openbrush::implementation(PSP22)]
#[openbrush::contract]
pub mod d9_usdt {
    use openbrush::{ traits::Storage, contracts::psp22::PSP22Error };
    use scale::{ Decode, Encode };
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        ApprovalError,
    }

    #[ink(event)]
    pub struct D9USDTTransfer {
        #[ink(topic)]
        from: AccountId,
        #[ink(topic)]
        to: AccountId,
        amount: Balance,
    }

    #[ink(storage)]
    #[derive(Default, Storage)]
    pub struct D9USDT {
        #[storage_field]
        psp22: psp22::Data,
    }

    impl D9USDT {
        #[ink(constructor)]
        pub fn new(initial_supply: Balance) -> Self {
            let mut _instance = Self::default();
            psp22::Internal
                ::_mint_to(&mut _instance, Self::env().caller(), initial_supply)
                .expect("Should mint");
            _instance
        }

        #[ink(message)]
        pub fn approve(
            &mut self,
            owner: AccountId,
            spender: AccountId,
            amount: Balance
        ) -> Result<(), PSP22Error> {
            psp22::Internal::_approve_from_to(self, owner, spender, amount)
        }

        #[ink(message)]
        pub fn transfer_from(
            &mut self,
            from: AccountId,
            to: AccountId,
            amount: Balance
        ) -> Result<(), PSP22Error> {
            psp22::Internal::_transfer_from_to(self, from, to, amount, [0u8].to_vec())
        }
    }
}
