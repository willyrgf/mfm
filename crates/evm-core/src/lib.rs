#![warn(missing_docs)]
//! Low-level EVM ABI, hex, and RLP helpers used by runtime and collector crates.
//!
//! This crate keeps encoding and decoding concerns separate from transport code so higher layers
//! can assemble RPC requests, contract calls, and byte-oriented payloads without reimplementing
//! Ethereum primitives.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_core::abi::function_selector;
//! use mfm_evm_core::encoding::{encode_erc20_decimals, normalize_address};
//!
//! let selector = function_selector("balanceOf", &["address".to_string()]);
//! assert_eq!(selector, [0x70, 0xa0, 0x82, 0x31]);
//! assert_eq!(encode_erc20_decimals(), "0x313ce567");
//! assert_eq!(
//!     normalize_address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")?,
//!     "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
//! );
//! # Ok::<(), mfm_evm_core::util_error::UtilError>(())
//! ```

/// ABI parsing, selector derivation, and argument encoding helpers.
pub mod abi;
/// Ethereum JSON-RPC quantity, address, and ERC-20 call encoding helpers.
pub mod encoding;
/// Hex-string normalization and decoding helpers.
pub mod hex;
/// Minimal Recursive Length Prefix (RLP) encoding helpers.
pub mod rlp;
/// Lightweight error type shared by the utility modules.
pub mod util_error;
