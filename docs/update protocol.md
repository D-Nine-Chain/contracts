first get the old code hash. this will be useful for returning to a previous version of the contract if anything goes
wrong in the update.

`cargo contract info --url $NETWORK --contract $CONTRACT_NAME`

upload the new code to the chain:

`cargo contract upload --url $NETWORK --suri $PRIVATE_KEY -x`

then use the setCode function on the contract as normal.
