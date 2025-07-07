# D9 Contracts Project Memory

## Ink Version
This project uses ink = { version = "4.3.0", default-features = false }

## Project Structure
- `d9-core/` - Core modules directory containing:
  - `prism/` - Prism module
  - `safety/` - Safety module  
  - `d9-environment/` - Environment definitions for D9
  - `chain-extension/` - Chain extension definitions
  - `d9-common-types/` - Common type definitions (includes RuntimeError)

## Key Information
- All modules use ink version 4.3.0
- RuntimeError enum has been extracted to d9-common-types for shared use
- D9Environment struct is in d9-core/d9-environment
- Chain extension trait and implementation are in d9-core/chain-extension