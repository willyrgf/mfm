//! Minimal Recursive Length Prefix (RLP) encoders.
//!
//! These helpers are intentionally small and focused on the byte-string/list encodings used by
//! the transaction-building codepaths in this repository.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_core::rlp::{rlp_encode_bytes, rlp_encode_list};
//!
//! assert_eq!(rlp_encode_bytes(b"dog"), vec![0x83, b'd', b'o', b'g']);
//! assert_eq!(rlp_encode_list(&[b"cat".to_vec(), b"dog".to_vec()])[0], 0xc8);
//! ```

/// Removes all leading zero bytes from a big-endian integer encoding.
pub fn trim_leading_zero_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut idx = 0usize;
    while idx < bytes.len() && bytes[idx] == 0 {
        idx += 1;
    }
    bytes[idx..].to_vec()
}

/// Encodes a `u64` as the shortest possible big-endian byte string.
pub fn u64_to_min_be(value: u64) -> Vec<u8> {
    unsigned_to_min_be(value as u128)
}

/// Encodes a `u128` as the shortest possible big-endian byte string.
pub fn u128_to_min_be(value: u128) -> Vec<u8> {
    unsigned_to_min_be(value)
}

fn unsigned_to_min_be(mut value: u128) -> Vec<u8> {
    if value == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    while value > 0 {
        out.push((value & 0xff) as u8);
        value >>= 8;
    }
    out.reverse();
    out
}

/// Encodes a `usize` as the shortest possible big-endian byte string.
///
/// Unlike the integer helpers above, `0` is encoded as a single zero byte because the value is
/// primarily used as an RLP length prefix.
pub fn usize_to_min_be(value: usize) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }

    unsigned_to_min_be(value as u128)
}

/// RLP-encodes a single byte string.
pub fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return vec![bytes[0]];
    }

    let mut out = Vec::new();
    if bytes.len() <= 55 {
        out.push(0x80 + bytes.len() as u8);
        out.extend_from_slice(bytes);
        return out;
    }

    let len_bytes = usize_to_min_be(bytes.len());
    out.push(0xb7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(bytes);
    out
}

/// RLP-encodes a list of raw byte strings.
///
/// Each item is individually encoded as a byte string before the outer list prefix is applied, so
/// callers should pass raw payloads here rather than pre-encoded RLP items.
pub fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(&rlp_encode_bytes(item));
    }

    rlp_encode_list_payload(payload)
}

fn rlp_encode_list_payload(payload: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
        out.extend_from_slice(&payload);
        return out;
    }

    let len_bytes = usize_to_min_be(payload.len());
    out.push(0xf7 + len_bytes.len() as u8);
    out.extend_from_slice(&len_bytes);
    out.extend_from_slice(&payload);
    out
}

/// RLP-encodes a list whose items are already RLP-encoded.
pub fn rlp_encode_list_preencoded(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(item);
    }

    rlp_encode_list_payload(payload)
}

#[cfg(test)]
#[path = "rlp_tests.rs"]
mod tests;
