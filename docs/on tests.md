## on tests

### Unit tests

`cargo tests`

### End to End Tests

for end to end tests make sure that the environment is `DefaultEnvironment`
`cargo test --features e2e-tests`

there is an error

### Error

if you encounter this error:

```
    ERROR: An unexpected panic function import was found in the contract Wasm.
            This typically goes back to a known bug in the Rust compiler:
            https://github.com/rust-lang/rust/issues/78744

            As a workaround try to insert `overflow-checks = false` into your `Cargo.toml`.
            This will disable safe math operations, but unfortunately we are currently not
            aware of a better workaround until the bug in the compiler is fixed.


            ERROR: An unexpected import function was found in the contract Wasm: _ZN4core9panicking5panic17h41ab539aad567d64E.
            Import funtions must either be prefixed with 'memory', or part of a module prefixed with 'seal'

            Ignore with `--skip-wasm-validation`

```

to run just convert `Perbill` to `Percent` in the `d9-burn-mining` contract.
do this only for integration tests since the numerical results will be wrong.
