# mfm-transports-evm-dcv

Typed EVM deploy/configure/validate runners and replay verifier.

This crate binds certified EVM DCV state descriptors to live side-effect/read runners. It stages
protected invocation artifacts, commits typed side-effect evidence through runtime/store APIs, and
verifies replay from recorded facts, receipts, confirmations, and typed artifacts.

Live RPC endpoints and auth metadata are runtime-only capability configuration. Replay must not use
live RPC sources.
