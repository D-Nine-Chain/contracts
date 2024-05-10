#![cfg_attr(not(feature = "std"), no_std, no_main)]

#[openbrush::implementation(PSP22)]
#[openbrush::contract]
pub mod d9_usdt {
    use openbrush::{contracts::psp22::PSP22Error, traits::Storage};
    use scale::{Decode, Encode};
    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        ApprovalError,
    }

    #[ink(event)]
    pub struct Approval {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        spender: AccountId,
        amount: u128,
    }

    // (3)
    #[ink(event)]
    pub struct Transfer {
        #[ink(topic)]
        from: Option<AccountId>,
        #[ink(topic)]
        to: Option<AccountId>,
        value: u128,
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
            psp22::Internal::_mint_to(&mut _instance, Self::env().caller(), initial_supply)
                .expect("Should mint");
            _instance
        }
    }

    impl D9USDT {
        #[ink(message)]
        pub fn transfer(
            &mut self,
            to: AccountId,
            value: u128,
            _data: Vec<u8>,
        ) -> Result<(), PSP22Error> {
            psp22::Internal::_transfer_from_to(self, self.env().caller(), to, value, _data)?; // Update!
            self.env().emit_event(Transfer {
                from: Some(self.env().caller()),
                to: Some(to),
                value,
            });
            Ok(())
        }

        #[ink(message)]
        pub fn transfer_from(
            &mut self,
            from: AccountId,
            to: AccountId,
            value: Balance,
            _data: Vec<u8>,
        ) -> Result<(), PSP22Error> {
            let allowance = psp22::Internal::_allowance(self, &from, &to);
            if allowance < value {
                return Err(PSP22Error::InsufficientAllowance);
            }
            psp22::Internal::_transfer_from_to(self, from, to, value, _data)?; // Update!
            self.env().emit_event(Transfer {
                from: Some(from),
                to: Some(to),
                value,
            });
            Ok(())
        }
    }
}
