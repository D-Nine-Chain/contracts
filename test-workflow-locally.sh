#!/bin/bash

# Test the workflow steps locally
echo "Testing PR contract check workflow locally..."

# Simulate changed contracts
CONTRACTS="mining-pool"

for contract in $CONTRACTS; do
    echo "Testing $contract..."
    cd "$contract" || exit 1
    
    echo "1. cargo check --all-features"
    cargo check --all-features || { echo "cargo check failed"; exit 1; }
    
    echo "2. cargo test"
    cargo test || { echo "cargo test failed"; exit 1; }
    
    echo "3. cargo clippy -- -D warnings"
    cargo clippy -- -D warnings || { echo "cargo clippy failed"; exit 1; }
    
    echo "4. cargo contract build --release"
    cargo contract build --release || { echo "cargo contract build failed"; exit 1; }
    
    cd ..
done

echo "All checks passed!"