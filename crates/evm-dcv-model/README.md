# mfm-evm-dcv-model

Pure deploy/configure/validate model and ABI preparation helpers.

This crate owns the serializable DCV model surface and deterministic preparation helpers:

- contract artifact wrappers
- configure/read/event assertion config types
- ABI parsing, calldata construction, and constructor payload preparation
- validation assertion preparation
- small normalization helpers shared by planners and runtime states

It may depend on `mfm-evm-core`, but it must not depend on runtime, state, op, transport, storage,
or machine crates.
