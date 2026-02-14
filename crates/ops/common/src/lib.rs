//! Shared building blocks for operation crates.
//!
//! This crate sits between the machine/runtime layer and domain operation crates:
//! reusable states + common helpers live here.

pub mod abi;
pub mod ctx;
pub mod errors;
pub mod evm_encoding;
pub mod hex;
pub mod idempotency;
pub mod keystore_tx;
pub mod local_io;
pub mod output;
pub mod rlp;
pub mod rpc;
pub mod states;
pub mod test_support;
pub mod util_error;
