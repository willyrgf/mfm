//! Canonical JSON syntax bounds.
//!
//! These are the global grammar budgets every canonical decoder shares. Bounds
//! that describe a domain rather than the syntax — batch sizes, portable
//! framing, fact scans, provider payloads — belong to the owner that enforces
//! them, not here.

/// Maximum items in one canonical array.
pub const MAX_ARRAY_ITEMS: usize = 1048576;

/// Maximum characters in one base64url-without-padding string.
pub const MAX_BASE64URL_CHARACTERS: usize = 22369622;

/// Maximum bytes in one canonical JSON document.
pub const MAX_CANONICAL_JSON_BYTES: usize = 33554432;

/// Maximum nested array/object depth in one canonical JSON document.
pub const MAX_CANONICAL_JSON_DEPTH: usize = 64;

/// Maximum UTF-8 bytes in one canonical object key.
pub const MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES: usize = 1048576;

/// Maximum entries in one canonical object.
pub const MAX_OBJECT_ENTRIES: usize = 1048576;

/// Maximum UTF-8 bytes in one canonical string.
///
/// The string budget is one byte below the largest root-string payload so a
/// string can exceed this bound while its containing document still remains
/// within [`MAX_CANONICAL_JSON_BYTES`].
pub const MAX_STRING_UTF8_BYTES: usize = MAX_CANONICAL_JSON_BYTES - 3;
