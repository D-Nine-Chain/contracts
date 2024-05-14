- first create network snapshot

in making snapshot, make sure `mainnet-archiver` is online and up to date. 

a successful snapshot will look like this:

```bash
stopping mainnet-archiver
2024-05-12 02:59:10 Essential task `transaction-pool-task-0` failed. Shutting down service.    
2024-05-12 02:59:10 Essential task `txpool-background` failed. Shutting down service.    
2024-05-12 02:59:10 Exporting raw state...    
2024-05-12 02:59:10 Essential task `transaction-pool-task-1` failed. Shutting down service.    
2024-05-12 02:59:10 Essential task `basic-block-import-worker` failed. Shutting down service.    
2024-05-12 02:59:10 Generating new chain spec...    
copying file to local
snapshot-main-spec.json                              100%   14MB 407.3KB/s   00:34    
starting mainnet-archiver
```

- make sure testnet (runtime tester) is up to date
- test on test network

## uploading to main
get existing code hash. this will be useful if need to revert.
`cargo contract info --url $D9_MAINNET --contract $CONTRACT_NAME`

check again if necessary on polkadot

upload the new contract 

uploaded code hashes should be the same for main and testnet (if test net is an exact copy of main net)

(execute from folder containing contract)

run once to make sure it works. 

`cargo contract upload --url $D9_MAINNET --suri $D9_CONTRACT_WORKER`

run again to execute 

`cargo contract upload --url $D9_MAINNET --suri $D9_CONTRACT_WORKER -x`

check again on polkadot website that it is there should look like this:

```bash
{
  instructionWeightsVersion: 4
  initial: 2
  maximum: 16
  code: 0x0061736d0100000001a5011960027f7f0060017f017f60017f0060037f7f7f0060047f7f7e7e017f60047f7f7f7f017f60037f7f7f017f60057f7e...
  determinism: Enforced
}
```
                
run setcode on contract            