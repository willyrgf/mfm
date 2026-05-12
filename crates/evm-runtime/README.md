# mfm-evm-runtime

Reusable EVM runtime states for MFM workflows.

This crate packages shared `State` implementations for on-chain reads and writes so ops can stay planning-only and binaries can stay transport-only.

Write states use durable signed transaction intents: they prepare and record a non-secret
`TxIntentV1` fact before broadcasting the exact raw transaction through `rpc.control`. Node-managed
unsigned transaction submission is intentionally rejected by production write states.

Deploy/configure/validate ABI handling flows through `mfm_evm_runtime::dcv`, which delegates
low-level parsing and calldata construction to `mfm-evm-core`. The supported argument surface is
`address`, `bool`, `bytes1` through `bytes32`, dynamic `bytes`, `string`, and `int`/`uint` widths
from 8 through 256 bits. Unsupported types return explicit validation errors rather than being
silently skipped during overload resolution.
