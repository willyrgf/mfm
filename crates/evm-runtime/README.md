# mfm-evm-runtime

Reusable EVM runtime states for MFM workflows.

This crate packages shared `State` implementations for on-chain reads and writes so ops can stay planning-only and binaries can stay transport-only.

Write states use durable signed transaction intents: they prepare and record a non-secret
`TxIntentV1` fact before broadcasting the exact raw transaction through `rpc.control`. Node-managed
unsigned transaction submission is intentionally rejected by production write states.
