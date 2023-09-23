#![cfg_attr(not(feature = "std"), no_std)]

#[ink::contract(env = D9Environment)]
mod d9_burn_mining {
    use d9_burn_common::{ Account, Error, D9Environment, BurnContractInterface };
    use ink::storage::Mapping;
    use sp_arithmetic::Percent;
    //  use d9_chain_extension::D9Environment;
    use ink::env::call::{ build_call, ExecutionInput, Selector };

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct D9burnMining {
        ///total amount of tokens burned so far globally
        ///
        /// amount should be denominated in base currency value and not subunit value
        total_amount_burned: Balance,
        ///the contract that maintains account data for an account with respect to N different
        /// burn contracts
        master_portfolio_contract: AccountId,
        /// mapping of account ids to account data
        accounts: Mapping<AccountId, Account>,
    }

    impl BurnContractInterface for D9burnMining {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(master_portfolio_contract: AccountId) -> Self {
            //todo define a struct to hold account data for what was burned and what was paid out
            Self {
                total_amount_burned: Default::default(),
                master_portfolio_contract,
                accounts: Default::default(),
            }
        }

        /// only executable by a user account
        /// burns the amount of tokens sent to the contract and
        /// updates the account data on the portfolio contract
        #[ink(message, payable)]
        pub fn user_execute(&mut self) -> Result<Account, Error> {
            let caller = self.env().caller();
            if caller == self.master_portfolio_contract {
                return Err(Error::UsePortfolioExecuteFunction);
            }
            let amount: Balance = self.env().transferred_value();
            self.env()
                .transfer(self.master_portfolio_contract, amount)
                .expect("Transfer to master portfolio failed.");
            let updated_account = self._burn(caller, amount);
            match updated_account {
                Ok(account) => {
                    let portfolio_update_result = self._update_portfolio(account.clone());
                    match portfolio_update_result {
                        Ok(_) => { Ok(account) }

                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
                Err(e) => Err(e),
            }
        }
        /// only executable by the portfolio contract that is define at instantiation
        /// processes the burn and updates the account data on the portfolio contract
        #[ink(message)]
        pub fn portfolio_execute(
            &mut self,
            account_id: AccountId,
            burn_amount: Balance
        ) -> Result<Balance, Error> {
            if self.env().caller() != self.master_portfolio_contract {
                return Err(Error::RestrictedFunction);
            }
            let account = self._burn(account_id, burn_amount);
            account.balance_due.saturating_sub(account.balance_paid)
        }

        fn _burn(&mut self, account_id: AccountId, amount: Balance) -> Result<Account, Error> {
            if amount < 100_000_000 {
                return Err(Error::BurnAmountBelowThreshold);
            }
            self.env().transfer(self.master_portfolio_contract, amount).expect("Transfer failed.");
            self.total_amount_burned = self.total_amount_burned.saturating_add(amount);
            let maybe_account: Option<Account> = self.accounts.get(&account_id);
            let account = match maybe_account {
                Some(mut account) => {
                    account.amount_burned = account.amount_burned.saturating_add(amount);
                    account.last_burn = self.env().block_timestamp();
                    account.balance_due = account.balance_due.saturating_add(
                        amount.saturating_mul(3)
                    );
                    self.accounts.insert(self.env().caller(), &account);
                    account
                }
                None => {
                    let account = Account {
                        amount_burned: amount,
                        balance_due: amount.saturating_mul(3),
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
                amount: amount,
            });
            Ok(account)
        }

        #[ink(message)]
        pub fn process_withdrawal(
            &self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<Account, Error> {
            if self.env().caller() != self.master_portfolio_contract {
                return Err(Error::RestrictedFunction);
            }
            let result = self._process_withdrawal(account_id, amount);
            match result {
                Ok(account) => {
                    let portfolio_update_result = self._update_portfolio(account.clone());
                    match portfolio_update_result {
                        Ok(_) => { Ok(account) }

                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
                Err(e) => Err(e),
            }
        }

        #[ink(message)]
        pub fn request_withdrawal(&mut self, amount: Balance) -> Result<Account, Error> {
            let result = self._process_withdrawal(self.env().caller(), amount);
            match result {
                Ok(account) => {}
                Error => Err(e),
            }
        }

        /// A private function that handles the actual withdrawal mechanics for a given account.
        ///
        /// This function is intended to be called internally by `process_withdrawal`. Its primary
        /// responsibility is to handle the underlying withdrawal process. It first validates the withdrawal
        /// request based on certain conditions (e.g., checking the time since the last withdrawal) and
        /// calculates the withdrawal allowance. Subsequently, it updates the account's information in storage
        /// and sends a payout request to the portfolio contract.
        ///
        /// # Parameters
        ///
        /// - `account_id`: The ID of the account initiating the withdrawal.
        /// - `amount`: The desired amount to be withdrawn.
        ///
        /// # Returns
        ///
        /// - `Result<Account, Error>`: If the withdrawal is successful, returns the updated account details.
        ///   If the withdrawal is unsuccessful, an error will be returned explaining the cause.
        ///
        /// # Errors
        ///
        /// Potential errors include:
        /// - `NoAccountFound`: If no account exists for the given `account_id`.
        /// - `EarlyWithdrawalAttempt`: If an attempt is made to withdraw before the allowed period.
        /// - Errors resulting from `_request_payout`.
        ///
        /// # Notes
        ///
        /// The function enforces a cool-down period between consecutive withdrawals. If the last withdrawal
        /// was within the last 24 hours, the function will reject the withdrawal request.
        ///
        /// If after processing the withdrawal, the `balance_due` becomes zero, the account is removed
        /// from storage.
        fn _process_withdrawal(
            &mut self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<Account, Error> {
            pub const DAY: Timestamp = 86_400;
            let maybe_account: Option<Account> = self.accounts.get(&account_id);
            if maybe_account.is_none() {
                return Err(Error::NoAccountFound);
            }
            let account = maybe_account.unwrap();

            //validate withdrawal
            let days_since_last_withdraw = self
                .env()
                .block_timestamp()
                .saturating_sub(account.last_withdrawal)
                .saturating_div(DAY);
            if days_since_last_withdraw == 0 {
                return Err(Error::EarlyWithdrawalAttempt);
            }

            let withdraw_allowance = self._calculate_withdrawal(&account);
            account.balance_paid = account.balance_paid.saturating_add(withdraw_allowance);
            account.last_withdrawal = self.env().block_timestamp();
            account.balance_due = account.balance_due.saturating_sub(withdraw_allowance);
            self.accounts.insert(caller, &account);

            //request payout from portfolio contract
            let payout_result = self._request_payout(account_id, withdraw_allowance);
            match payout_result {
                Ok(_) => {
                    if account.balance_due == 0 {
                        let account_clone: Account = account.clone();
                        self.accounts.remove(&caller);
                        return Ok(account_clone);
                    }
                    Ok(account)
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        fn _calculate_withdrawal(&self, &account: Account) -> Balance {
            let daily_return_percent = self._get_return_percent();
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
            withdraw_allowance
        }

        fn _request_payout(
            &self,
            account_id: AccountId,
            amount: Balance
        ) -> Result<Account, Error> {
            let portfolio_payout_result = build_call::<D9Environment>()
                .call(self.master_portfolio_contract)
                .gas_limit(0)
                .exec_input(
                    ExecutionInput::new(Selector::new(ink::selector_bytes!("request_payout")))
                        .push_arg(account_id)
                        .push_arg(amount)
                )
                .returns::<Result<(), Error>>()
                .invoke();
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
        fn _get_return_percent(&self) -> Percent {
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
