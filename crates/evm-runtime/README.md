# mfm-evm-runtime

Reusable EVM runtime states for MFM workflows.

This crate packages shared `State` implementations for on-chain reads and writes so ops can stay planning-only and binaries can stay transport-only.

EVM ownership is split by boundary:

- `mfm-evm-core` owns pure ABI, hex, RLP, and transaction support primitives.
- `mfm-evm-dcv-model` owns pure deploy/configure/validate models and calldata preparation.
- `mfm-collectors-local-evm` owns the typed `local.evm.*` signer IO client over `IoProvider`.
- `mfm-transports-local-evm` owns live local signing, private-key environment handling, and zeroized secret material.
- `mfm-evm-runtime` owns reusable EVM read/write states, `rpc.control` helpers, signed transaction intent recording, protected raw-transaction capabilities, and managed broadcast/read behavior.
- EVM op crates own graph assembly only.

Write states use durable signed transaction intents: they record non-secret `TxIntentV1` metadata
as a normal fact, store the signed raw transaction bytes through the protected artifact path, and
then broadcast the protected capability through `rpc.control`. Node-managed unsigned transaction
submission is intentionally rejected by production write states.

Deploy/configure/validate ABI handling flows through `mfm-evm-dcv-model`, which delegates
low-level parsing and calldata construction to `mfm-evm-core`. The supported argument surface is
`address`, `bool`, `bytes1` through `bytes32`, dynamic `bytes`, `string`, and `int`/`uint` widths
from 8 through 256 bits. Unsupported types return explicit validation errors rather than being
silently skipped during overload resolution.
