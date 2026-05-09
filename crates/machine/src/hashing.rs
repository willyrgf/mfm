//! Canonical JSON and `ArtifactId` hashing helpers.
//!
//! Policy:
//! - Structured data MUST be hashed as canonical JSON bytes (RFC 8785 / JCS-style).
//! - Hashed JSON MUST NOT contain floats (fractional numbers); use integer-scaled values or strings instead.

use crate::errors::{ErrorCategory, ErrorInfo, StorageError};
use crate::ids::ArtifactId;
use crate::ids::ErrorCode;
use crate::stores::{ArtifactKind, ArtifactStore};

/// Errors returned when a JSON value cannot participate in canonical hashing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalJsonError {
    /// A floating-point number was encountered in a value that must be canonically hashable.
    FloatNotAllowed,
    /// The value appears to contain secret-bearing fields and cannot be persisted or hashed.
    SecretsNotAllowed,
}

impl std::fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanonicalJsonError::FloatNotAllowed => {
                write!(
                    f,
                    "floats are not allowed in canonical-json-hashed structures"
                )
            }
            CanonicalJsonError::SecretsNotAllowed => {
                write!(
                    f,
                    "secrets are not allowed in persisted canonical-json structures"
                )
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

/// Serialize a `serde_json::Value` into canonical JSON bytes.
///
/// Target semantics: RFC 8785 (JCS-style) canonicalization.
/// Additional constraint: floats are rejected.
pub fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, CanonicalJsonError> {
    if crate::secrets::json_contains_secrets(value) {
        return Err(CanonicalJsonError::SecretsNotAllowed);
    }
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

/// Compute the content-addressed `ArtifactId` for raw bytes (SHA-256 lowercase hex).
pub fn artifact_id_for_bytes(bytes: &[u8]) -> ArtifactId {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    ArtifactId::must_new(hex::encode(digest.as_ref()))
}

/// Compute the content-addressed `ArtifactId` for structured JSON (canonical JSON bytes + SHA-256).
pub fn artifact_id_for_json(value: &serde_json::Value) -> Result<ArtifactId, CanonicalJsonError> {
    Ok(artifact_id_for_bytes(&canonical_json_bytes(value)?))
}

fn storage_corruption(code: &'static str, message: &'static str) -> StorageError {
    StorageError::Corruption(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.to_string(),
        details: None,
    })
}

/// Verifies bytes returned for an artifact id still match the content address.
pub fn verify_artifact_bytes(id: &ArtifactId, bytes: &[u8]) -> Result<(), StorageError> {
    let actual = artifact_id_for_bytes(bytes);
    if &actual != id {
        return Err(storage_corruption(
            "artifact_content_address_mismatch",
            "artifact bytes did not match requested content address",
        ));
    }
    Ok(())
}

/// Stores an immutable content-addressed artifact and verifies the store-returned id.
pub async fn put_artifact_verified(
    store: &dyn ArtifactStore,
    kind: ArtifactKind,
    bytes: Vec<u8>,
) -> Result<ArtifactId, StorageError> {
    let expected = artifact_id_for_bytes(&bytes);
    let actual = store.put(kind, bytes).await?;
    if actual != expected {
        return Err(storage_corruption(
            "artifact_put_id_mismatch",
            "artifact store returned an id that did not match the stored bytes",
        ));
    }
    Ok(actual)
}

fn write_canonical_json(
    value: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), CanonicalJsonError> {
    match value {
        serde_json::Value::Null => out.extend_from_slice(b"null"),
        serde_json::Value::Bool(true) => out.extend_from_slice(b"true"),
        serde_json::Value::Bool(false) => out.extend_from_slice(b"false"),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.extend_from_slice(i.to_string().as_bytes());
            } else if let Some(u) = n.as_u64() {
                out.extend_from_slice(u.to_string().as_bytes());
            } else {
                return Err(CanonicalJsonError::FloatNotAllowed);
            }
        }
        serde_json::Value::String(s) => {
            // Delegate escaping/quoting to serde_json.
            let json = serde_json::to_string(s).expect("string serialization must not fail");
            out.extend_from_slice(json.as_bytes());
        }
        serde_json::Value::Array(a) => {
            out.push(b'[');
            for (idx, v) in a.iter().enumerate() {
                if idx != 0 {
                    out.push(b',');
                }
                write_canonical_json(v, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(m) => {
            out.push(b'{');

            // RFC 8785 (JCS) requires lexicographic ordering of member names.
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();

            for (idx, key) in keys.iter().enumerate() {
                if idx != 0 {
                    out.push(b',');
                }
                let k = serde_json::to_string(key).expect("string serialization must not fail");
                out.extend_from_slice(k.as_bytes());
                out.push(b':');
                write_canonical_json(&m[*key], out)?;
            }

            out.push(b'}');
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn canonical_json_vectors_basic() {
        let v = serde_json::json!({"b": 1, "a": 2});
        assert_eq!(canonical_json_bytes(&v).unwrap(), br#"{"a":2,"b":1}"#);

        let v = serde_json::json!({"arr": [true, null, 3], "obj": {"y": 2, "x": 1}});
        assert_eq!(
            canonical_json_bytes(&v).unwrap(),
            br#"{"arr":[true,null,3],"obj":{"x":1,"y":2}}"#
        );

        let v = serde_json::json!({"s": "a\nb"});
        assert_eq!(canonical_json_bytes(&v).unwrap(), br#"{"s":"a\nb"}"#);
    }

    #[test]
    fn canonical_json_rejects_floats() {
        let v = serde_json::json!({"x": 1.5});
        assert_eq!(
            canonical_json_bytes(&v).unwrap_err(),
            CanonicalJsonError::FloatNotAllowed
        );

        let v = serde_json::json!([1.0, 2]);
        assert_eq!(
            canonical_json_bytes(&v).unwrap_err(),
            CanonicalJsonError::FloatNotAllowed
        );
    }

    #[test]
    fn canonical_json_rejects_secrets() {
        let v = serde_json::json!({"mnemonic": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"});
        assert_eq!(
            canonical_json_bytes(&v).unwrap_err(),
            CanonicalJsonError::SecretsNotAllowed
        );
    }

    #[test]
    fn artifact_id_sha256_lowercase_hex() {
        // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let id = artifact_id_for_bytes(b"hello");
        assert_eq!(
            id.as_str(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(id.as_str().len(), 64);
        assert!(id
            .as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    struct WrongIdStore;

    #[async_trait]
    impl ArtifactStore for WrongIdStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId::must_new("f".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            unreachable!("put_artifact_verified does not read artifacts")
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            unreachable!("put_artifact_verified does not query existence")
        }
    }

    #[tokio::test]
    async fn verified_put_rejects_store_returned_wrong_id() {
        let err = put_artifact_verified(
            &WrongIdStore,
            ArtifactKind::Other("test".to_string()),
            b"payload".to_vec(),
        )
        .await
        .expect_err("wrong store-returned id must fail");

        match err {
            StorageError::Corruption(info) => assert_eq!(info.code.0, "artifact_put_id_mismatch"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn verify_artifact_bytes_rejects_mismatched_content() {
        let id = artifact_id_for_bytes(b"expected");
        let err = verify_artifact_bytes(&id, b"different")
            .expect_err("wrong bytes must not satisfy content address");

        match err {
            StorageError::Corruption(info) => {
                assert_eq!(info.code.0, "artifact_content_address_mismatch")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
