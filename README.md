#

---

# D9 Network Smart Contracts

The D9 Network is an innovative blockchain platform based on the Substrate framework that focuses on bringing advanced features and functionalities to its users. One of the primary features of the D9 Network is its capability to handle smart contracts, specifically crafted with the Rust programming language and the ink! framework.

## Introduction to D9 Contracts

Smart contracts on the D9 Network are designed to ensure efficiency, transparency, and security. By utilizing the Rust programming language, these contracts inherit the safety guarantees provided by the language's type system and ownership model. The ink! framework further adds specialized tools and functionalities tailored for blockchain contract development.

### Key Features:

1. **Safety Assured**: Rust's strong type system and memory management capabilities ensure contract safety.
2. **Optimized for Performance**: The D9 Network and ink! ensure that contracts are optimized for execution and resource usage.
3. **Rich Library Support**: Contracts can take advantage of the extensive set of libraries and packages available in the Rust ecosystem.
4. **Transparent Interactions**: The D9 Network ensures all contract interactions are transparent and verifiable, adding to the trust factor.

## Note on Development Environment

When developing for the D9 Network, especially with ink! contracts, it's essential to pay attention to the Rust version being used. We recommend using Rust version 1.69. It's advised for developers to include a `rust-toolchain` file in their projects specifying this version. This ensures that the project will be built using a Rust version that's known to work seamlessly with ink! contracts on the D9 Network.

```plaintext
# rust-toolchain file content
1.69.0
```

With the correct Rust version and a keen understanding of the D9 Network's capabilities, developers can craft powerful, efficient, and reliable smart contracts for a wide array of applications.

## Building guide

make sure to use:
`cargo contract build`

to build all contracts.

## secrets using vlt

`vlt run -- env | grep D9_CONTRACT_WORKER

0x75fca36af004fc6239e7b63b2fcaeb1fb63ec7de025ca0e9112a0ecab2fb0354