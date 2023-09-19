#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract(env = D9Environment)]
mod d9burnmining {
    use ink::storage::Mapping;
    use sp_arithmetic::Percent;
    use scale::{ Decode, Encode };
    use d9_chain_extension::{ D9Environment };

    #[derive(Decode, Encode)]
    #[cfg_attr(
        feature = "std",
        derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            scale_info::TypeInfo,
            ink::storage::traits::StorageLayout
        )
    )]
    pub struct Account {
        amount_burned: Balance,
        balance_due: Balance,
        balance_paid: Balance,
        last_withdrawal: Timestamp,
        last_burn: Timestamp,
    }

    #[derive(Encode, Decode, Debug, PartialEq, Eq, Copy, Clone)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        /// indicates that the burn function was called without sending any tokens
        ZeroBurnAmount,
        BurnAmountBelowThreshold,

        NoAccountFound,
        /// account tried to withdraw within the 24 hour limit
        EarlyWithdrawalAttempt,
        /// this contract has insufficient funds.
        ContractBalanceTooLow,
    }
    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        ///
        /// amount should be denominated in base currency value and not subunit value
        total_amount_burned: Balance,
        pool_contract: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
    }

    #[ink(event)]
    pub struct BurnExecuted {
        /// initiator of of the burn
        #[ink(topic)]
        from: AccountId,
        ///amount of tokens burned
        #[ink(topic)]
        amount: Balance,
    }

    #[ink(event)]
    pub struct WithdrawalExecuted {
        /// initiator of of the burn
        #[ink(topic)]
        from: AccountId,
        ///amount of tokens burned
        #[ink(topic)]
        amount: Balance,
    }

    impl D9burnMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(pool_contract: AccountId) -> Self {
            //todo define a struct to hold account data for what was burned and what was paid out
            Self {
                total_amount_burned: Default::default(),
                pool_contract,
                accounts: Default::default(),
            }
        }

        /// A message that can be called on instantiated contracts.
        /// This one flips the value of the stored `bool` from `true`
        /// to `false` and vice versa.
        #[ink(message, payable)]
        pub fn burn(&mut self) -> Result<Account, Error> {
            let burn_amount = self.env().transferred_value();
            if burn_amount == 0 {
                return Err(Error::ZeroBurnAmount);
            } else if burn_amount < 100_000_000 {
                return Err(Error::BurnAmountBelowThreshold);
            }
            self.env().transfer(self.pool_contract, burn_amount).expect("Transfer failed.");
            self.total_amount_burned = self.total_amount_burned.saturating_add(burn_amount);
            let maybe_account: Option<Account> = self.accounts.get(&self.env().caller().clone());
            let account = match maybe_account {
                Some(mut account) => {
                    account.amount_burned = account.amount_burned.saturating_add(burn_amount);
                    account.last_burn = self.env().block_timestamp();
                    account.balance_due = account.balance_due.saturating_add(
                        burn_amount.saturating_mul(3)
                    );
                    self.accounts.insert(self.env().caller(), &account);
                    account
                }
                None => {
                    let account = Account {
                        amount_burned: burn_amount,
                        balance_due: burn_amount.saturating_mul(3),
                        balance_paid: 0,
                        last_withdrawal: 0,
                        last_burn: self.env().block_timestamp(),
                    };
                    self.accounts.insert(self.env().caller(), &account);
                    account
                }
            };
            self.env().emit_event(BurnExecuted {
                from: self.env().caller(),
                amount: burn_amount,
            });
            Ok(account)
        }

        #[ink(message)]
        pub fn withdraw(&mut self) -> Result<Account, Error> {
            pub const DAY: Timestamp = 86_400;
            let caller = self.env().caller();
            let maybe_account: Option<Account> = self.accounts.get(&caller);
            let mut account = match maybe_account {
                Some(account) => account,
                None => {
                    return Err(Error::NoAccountFound);
                }
            };
            let days_since_last_withdraw = self
                .env()
                .block_timestamp()
                .saturating_sub(account.last_withdrawal)
                .saturating_div(DAY);
            if days_since_last_withdraw == 0 {
                return Err(Error::EarlyWithdrawalAttempt);
            }
            let daily_return_percent = self.get_return_percent();
            let withdraw_allowance: Balance = {
                let allowance = daily_return_percent
                    .mul_floor(account.balance_due)
                    .saturating_mul(days_since_last_withdraw as u128);

                if allowance > account.balance_due {
                    account.balance_due
                } else {
                    allowance
                }
            };
            if self.env().balance() < withdraw_allowance {
                return Err(Error::ContractBalanceTooLow);
            }
            //todo if balance due zero close account
            self.env().transfer(caller, withdraw_allowance).expect("Transfer failed.");
            account.balance_paid = account.balance_paid.saturating_add(withdraw_allowance);
            account.last_withdrawal = self.env().block_timestamp();
            account.balance_due = account.balance_due.saturating_sub(withdraw_allowance);
            self.accounts.insert(caller, &account);
            self.env().emit_event(WithdrawalExecuted {
                from: caller,
                amount: withdraw_allowance,
            });

            if account.balance_due == 0 {
                let account_clone: Account = account.clone();
                self.accounts.remove(&caller);
                return Ok(account_clone);
            }
            Ok(account)
        }

        /// the returned percent is used for an accounts return based on the amount burned
        ///
        /// This function calculates the return percentage based on the total amount burned
        /// within the contract. The return percentage starts at 0.8% and is reduced by half
        /// for every 100_000_000_000_000 units over the first threshold of 200_000_000_000_000.
        ///
        /// # Parameters:
        ///
        /// - `&self`: A reference to the instance of the ink! contract.
        ///
        /// # Returns:
        ///
        /// Returns a `Percent` value representing the return percentage.
        ///
        fn get_return_percent(&self) -> Percent {
            let first_threshold_amount: Balance = 200_000_000_000_000;
            let mut percentage: f64 = 0.008;

            if self.total_amount_burned <= first_threshold_amount {
                return Percent::from_float(percentage);
            }

            let excess_amount: u128 =
                self.total_amount_burned.saturating_sub(first_threshold_amount);
            let reductions: u128 = excess_amount
                .saturating_div(100_000_000_000_000)
                .saturating_add(1);

            for _ in 0..reductions {
                percentage /= 2.0;
            }
            Percent::from_float(percentage)
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
