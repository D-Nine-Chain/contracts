# Market-Maker (AMM) Integration Report

## Overview
This report analyzes which contracts in the D9 ecosystem integrate with the market-maker contract (referred to as AMM - Automated Market Maker) and details the specific functions being called.

## Contracts Integrating with Market-Maker

### 1. Merchant Mining Contract (`merchant-mining/lib.rs`)

**Integration Purpose**: The merchant mining contract uses the market-maker for currency conversions when processing merchant payments and distributing rewards.

**Storage Field**:
```rust
amm_contract: Option<AccountId>  // Line 36
```

**Functions Called**:

#### `get_d9(usdt_amount: Balance)`
- **Purpose**: Converts USDT to D9 tokens
- **Used in**: `add_d9_via_market_trade()` function (Line 283)
- **Context**: Allows merchants to add D9 liquidity by first converting USDT to D9
- **Call Pattern**:
  ```rust
  let selector = selector_bytes!("get_d9");
  build_call::<DefaultEnvironment>()
      .call(amm_contract)
      .transferred_value(0)
      .exec_input(ExecutionInput::new(Selector::new(selector)).push_arg(usdt))
      .returns::<Result<Balance, Error>>()
      .invoke()
  ```

#### `get_usdt()`
- **Purpose**: Converts D9 to USDT tokens
- **Used in**: `renew_subscription()` function (Line 214)
- **Context**: Converts merchant's D9 balance to USDT for subscription renewal
- **Call Pattern**: Similar to above but with D9 value transferred in the call

#### `estimate_exchange(direction: Direction, amount: Balance)`
- **Purpose**: Gets exchange rate estimates without performing actual swap
- **Used in**: 
  - `add_d9_via_market_trade()` (Line 280) - to estimate D9 output
  - `renew_subscription()` (Line 207) - to calculate required D9 for USDT conversion
- **Context**: Pre-calculates conversion amounts for validation and user information

### 2. Mining Pool Contract (`mining-pool/lib.rs`)

**Integration Purpose**: Uses the market-maker for exchange rate calculations in the mining rewards system.

**Storage Field**:
```rust
amm_contract: Option<AccountId>  // Line 29
```

**Functions Called**:

#### `estimate_exchange(direction: Direction, amount: Balance)`
- **Purpose**: Gets current exchange rates for reward calculations
- **Used in**: `distribute()` function (Line 335)
- **Context**: Calculates USDT equivalent values for D9 rewards
- **Note**: Only used if AMM contract is set, otherwise defaults to 1:100,000 ratio

**Admin Functions**:
- `change_amm_contract(new_amm: AccountId)` - Updates the AMM contract address

## Contracts NOT Integrating with Market-Maker

The following contracts do not have any direct integration with the market-maker:

1. **Main Pool Contract** - Manages node rewards distribution
2. **Node Reward Contract** - Handles individual node rewards
3. **Cross-Chain Transfer Contract** - Manages cross-chain token transfers
4. **D9 Burn Mining Contract** - Handles token burning mechanics

## Integration Patterns

### Common Pattern
Both integrating contracts follow a similar pattern:
1. Store AMM contract address as optional field
2. Use cross-contract calls via `build_call` 
3. Handle both successful responses and errors
4. Provide admin functions to update AMM address

### Error Handling
Contracts gracefully handle cases where:
- AMM contract is not set (returns specific error)
- AMM call fails (propagates error)
- Insufficient liquidity or invalid amounts

### Security Considerations
- Only admin/root can update AMM contract address
- All AMM calls are made through proper cross-contract call patterns
- Value transfers are explicitly controlled

## Market-Maker Functions Summary

| Function | Parameters | Purpose | Used By |
|----------|-----------|---------|---------|
| `get_d9` | `usdt_amount: Balance` | Convert USDT to D9 | Merchant Mining |
| `get_usdt` | (payable with D9) | Convert D9 to USDT | Merchant Mining |
| `estimate_exchange` | `direction: Direction, amount: Balance` | Get exchange rate | Merchant Mining, Mining Pool |
| `add_liquidity` | Various | Add liquidity to pool | Not directly called |
| `remove_liquidity` | Various | Remove liquidity | Not directly called |

## Conclusion

The market-maker contract serves as a critical infrastructure component in the D9 ecosystem, providing automated token swapping functionality. Currently, only 2 out of 7 contracts integrate with it directly:

1. **Merchant Mining** - For payment processing and reward distribution
2. **Mining Pool** - For reward value calculations

The integration is optional in both cases, allowing the system to function without the AMM if needed, though with reduced functionality. The market-maker essentially acts as an on-chain DEX (Decentralized Exchange) facilitating D9/USDT swaps within the D9 ecosystem.