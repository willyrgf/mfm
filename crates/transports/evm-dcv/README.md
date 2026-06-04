# mfm-transports-evm-dcv

Typed EVM deploy/configure/validate runners and replay verifier.

This crate binds certified EVM DCV state descriptors to live side-effect/read runners. It stages
protected invocation artifacts, commits typed side-effect evidence through runtime/store APIs, and
verifies replay from recorded facts, receipts, confirmations, and typed artifacts.

Deploy and configure transactions are signed through non-secret keystore signer references carried
by typed config. The runner opens the referenced MFM keystore at submit time, signs the prepared
transaction transiently, and submits the raw transaction without storing raw signed bytes.

Live RPC endpoints, auth metadata, keystore paths, passwords, and raw signing material are
runtime-only capability configuration. Replay must not use live RPC sources or live signer access.
