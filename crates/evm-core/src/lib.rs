#![warn(missing_docs)]
//! Temporary EVM collector encoding helpers.
//!
//! Only the existing portfolio collector's ERC-20 and retained-evidence hex helpers remain here
//! until that collector moves to its final owner. Transaction construction, hashing, and encoding
//! are owned exclusively by `mfm-evm-signing` and Alloy.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_core::encoding::{encode_erc20_decimals, normalize_address};
//!
//! assert_eq!(encode_erc20_decimals(), "0x313ce567");
//! assert_eq!(
//!     normalize_address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")?,
//!     "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
//! );
//! # Ok::<(), mfm_evm_core::util_error::UtilError>(())
//! ```

/// Ethereum JSON-RPC quantity, address, and ERC-20 call encoding helpers.
pub mod encoding;
/// Hex-string normalization and decoding helpers.
pub mod hex;
/// Lightweight error type shared by the utility modules.
pub mod util_error;
