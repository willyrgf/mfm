# Alloy Adapter Layer

This directory contains adapter modules to help transition the MFM project from `ethers-rs` to `alloy`. 
The adapter layer is designed to be a temporary compatibility layer that allows for incremental 
migration of the codebase.

## Adapter Modules

### Types Adapter 

Located in `types.rs`, this module provides adapter types for common Ethereum primitives:
- `Address`: Wraps `alloy_primitives::Address` 
- `U256`: Wraps `alloy_primitives::U256`
- `Bytes`: Wraps `alloy_primitives::Bytes`

These adapter types implement common traits such as `FromStr` and `From` to make conversion between
ethers and alloy types easier.

### Provider Adapter

Located in `provider.rs`, this module provides an adapter for the RPC provider:
- `Provider`: Wraps `alloy_rpc_client::RpcClient<Http<Client>>`

It implements basic methods similar to the ethers provider API.

### Signer Adapter

Located in `signer.rs`, this module provides an adapter for wallet and signing operations:
- `LocalWallet`: Simple wallet implementation using `alloy_primitives::B256` for private keys

### Contracts Adapter

Located in `contracts.rs`, this module provides an adapter for generic contract interactions:
- `Contract`: Generic contract wrapper that handles RPC calls

### ERC20 Adapter

Located in `erc20.rs`, this module provides an adapter for ERC20 token interactions:
- `ERC20`: Specialized contract implementation for ERC20 tokens

## Migration Process

When migrating a module from ethers to alloy:

1. Replace imports from `ethers::types::*` with `crate::blockchain::adapter::types::*`
2. Update any zero address references from `Address::zero()` to `Address(address!("0000000000000000000000000000000000000000"))`
3. Update U256::zero() to use `U256::from(alloy_primitives::U256::ZERO)`
4. Replace checked arithmetic operations to handle the different API
5. Use our adapter types throughout the module

## Future Work

The adapter layer is intended to be temporary. Once the entire codebase has been migrated to use alloy,
we can gradually remove the adapter layer and use alloy types directly. This will require:

1. Replacing adapter types with direct alloy types
2. Updating the API usage to match alloy's native API
3. Removing the adapter layer entirely

For now, the adapter approach allows us to migrate the codebase incrementally while maintaining 
compatibility with existing code. 