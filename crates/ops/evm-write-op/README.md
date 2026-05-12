# mfm-op-evm-write

Thin planning ops for EVM deploy, configure, validate, and contract-set deployment.

The op crate validates config and wires reusable runtime states from `mfm-evm-runtime`. ABI parsing,
bytecode parsing, calldata construction, constructor argument encoding, and validation assertion
preparation are owned by `mfm-evm-runtime::dcv`, backed by `mfm-evm-core::abi`. Keep ABI semantics
there so planner validation and runtime execution share one contract.

Supported ABI argument types are documented in `mfm-evm-core::abi`. Unsupported types fail during
planning when an inline artifact is present, and fail in the runtime state when the artifact is
loaded from context.
