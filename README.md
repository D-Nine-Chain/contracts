# 

---

# D9 Network Smart Contracts

The D9 Network is an innovative blockchain platform based on the Substrate
framework that focuses on bringing advanced features and functionalities to its
users. One of the primary features of the D9 Network is its capability to handle
smart contracts, specifically crafted with the Rust programming language and the
ink! framework.

## Introduction to D9 Contracts

Smart contracts on the D9 Network are designed to ensure efficiency,
transparency, and security. By utilizing the Rust programming language, these
contracts inherit the safety guarantees provided by the language's type system
and ownership model. The ink! framework further adds specialized tools and
functionalities tailored for blockchain contract development.

### Key Features:

1. **Safety Assured**: Rust's strong type system and memory management
   capabilities ensure contract safety.
2. **Optimized for Performance**: The D9 Network and ink! ensure that contracts
   are optimized for execution and resource usage.
3. **Rich Library Support**: Contracts can take advantage of the extensive set
   of libraries and packages available in the Rust ecosystem.
4. **Transparent Interactions**: The D9 Network ensures all contract
   interactions are transparent and verifiable, adding to the trust factor.

## Note on Development Environment

When developing for the D9 Network, especially with ink! contracts, it's
essential to pay attention to the Rust version being used. We recommend using
Rust version 1.69. It's advised for developers to include a `rust-toolchain`
file in their projects specifying this version. This ensures that the project
will be built using a Rust version that's known to work seamlessly with ink!
contracts on the D9 Network.

```plaintext
# rust-toolchain file content
1.69.0
```

With the correct Rust version and a keen understanding of the D9 Network's
capabilities, developers can craft powerful, efficient, and reliable smart
contracts for a wide array of applications.

## Building guide

make sure to use: `cargo contract build`

to build all contracts.

## Contract Management with Makefile

The D9 contracts repository includes a comprehensive Makefile for managing
contract development, testing, and upload workflows. This automation ensures
consistency and reduces the risk of upload errors.

### Available Commands

#### Getting Help

```bash
make
# or
make help
```

#### Checking and Building Contracts

**Check all contracts:**

```bash
make check-all
```

This runs cargo check and tests for all contracts (market-maker,
merchant-mining, mining-pool).

**Check a specific contract:**

```bash
make check-contract CONTRACT=mining-pool
```

**Build a specific contract:**

```bash
make build-contract CONTRACT=mining-pool
```

This builds the contract in release mode and generates the .wasm file.

#### Upload Commands

**Upload to local development network:**

```bash
make upload-local CONTRACT=mining-pool
```

This uploads to `ws://localhost:9944` using the `//Alice` account.

**Upload to testnet:**

```bash
make upload-testnet CONTRACT=mining-pool SURI="your-secret-uri"
```

**Upload to mainnet:**

```bash
make upload-mainnet CONTRACT=mining-pool SURI="your-secret-uri"
```

⚠️ **Warning**: Mainnet upload requires:

- Being on the `main` branch
- Having an approved code hash from a merged PR
- Explicit confirmation during upload

**Upload code only (without instantiation):**

```bash
make upload-code CONTRACT=mining-pool NETWORK=testnet SURI="your-secret-uri"
```

#### Upload History

**View all upload history:**

```bash
make history
```

Shows complete upload history for all contracts with timestamps, networks, code
hashes, and uploaders.

**View history for specific contract:**

```bash
make history-contract CONTRACT=mining-pool
```

**View only latest uploads:**

```bash
make history-latest
```

Shows the most recent upload for each contract.

### Upload Security Features

1. **Branch Restrictions**:
   - Local: Any branch allowed
   - Testnet: Only `main`, `develop`, and `feature/*` branches
   - Mainnet: Only `main` branch

2. **Code Hash Verification**:
   - PR merges automatically record approved code hashes
   - Mainnet uploads verify the code hash matches approved versions

3. **Upload History**:
   - All uploads are recorded in `upload-history.json`
   - Includes timestamp, network, git commit, and uploader information

### Pre-upload Checks

Before any upload, the Makefile automatically:

1. Runs contract checks (cargo check)
2. Runs tests
3. Verifies storage layout hasn't changed
4. Builds the contract in release mode
5. Compares metadata for compatibility

### Example Workflow

```bash
# 1. Check your contract works correctly
make check-contract CONTRACT=mining-pool

# 2. Deploy to local network for testing
make deploy-local CONTRACT=mining-pool

# 3. After testing, deploy to testnet
make deploy-testnet CONTRACT=mining-pool SURI="//Alice"

# 4. Once approved via PR, deploy to mainnet
make deploy-mainnet CONTRACT=mining-pool SURI="your-production-key"

# 5. Check deployment history
make history

# 6. View specific contract history
make history-contract CONTRACT=mining-pool
```

### Troubleshooting

- **"CONTRACT not specified"**: Provide the CONTRACT parameter, e.g.,
  `CONTRACT=mining-pool`
- **"NETWORK not specified"**: Provide the NETWORK parameter
  (local/testnet/mainnet)
- **"SURI not specified"**: Provide your secret URI for signing transactions
- **"No approved hash found"**: For mainnet, ensure your code changes were
  merged via PR
- **"Branch not allowed"**: Switch to an allowed branch for your target network
