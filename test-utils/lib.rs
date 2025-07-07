#![cfg_attr(not(feature = "std"), no_std)]

type Timestamp = u64;

pub fn set_block_time(init_time: Timestamp) {
    ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(init_time);
    ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
}

///moves block forward by `move_forward_by` in milliseconds and moves chain forwards by one block
pub fn move_time_forward(move_forward_by: Timestamp) {
    let current_block_time: Timestamp = ink::env::block_timestamp::<ink::env::DefaultEnvironment>();
    let _ = ink::env::test::set_block_timestamp::<ink::env::DefaultEnvironment>(
        current_block_time + move_forward_by
    );
    let _ = ink::env::test::advance_block::<ink::env::DefaultEnvironment>();
}
