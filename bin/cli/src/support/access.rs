use std::path::Path;

use mfm_app::{PublicError, SecretCredential};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

/// Maximum accepted opaque credential size.
pub(crate) const MAX_ACCESS_TOKEN_BYTES: usize = mfm_app::MAX_SECRET_CREDENTIAL_BYTES;

/// Reads bounded opaque credential bytes without UTF-8 decoding, removing at most one terminal
/// LF or CRLF introduced by a token file.
pub(crate) async fn read_access_credential(
    path: Option<&Path>,
) -> Result<SecretCredential, PublicError> {
    let path = path.ok_or_else(PublicError::authentication_required)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| PublicError::authentication_required())?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_ACCESS_TOKEN_BYTES + 3));
    file.take((MAX_ACCESS_TOKEN_BYTES + 3) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| PublicError::authentication_required())?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.is_empty() || bytes.len() > MAX_ACCESS_TOKEN_BYTES {
        return Err(PublicError::authentication_required());
    }
    SecretCredential::new(std::mem::take(&mut *bytes))
        .map_err(|_| PublicError::authentication_required())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{read_access_credential, MAX_ACCESS_TOKEN_BYTES};

    #[tokio::test]
    async fn reader_accepts_opaque_boundary_bytes_and_rejects_one_more() {
        let directory = TempDir::new().expect("temporary token directory");
        for (name, suffix) in [
            ("boundary.token", Vec::new()),
            ("boundary-lf.token", vec![b'\n']),
            ("boundary-crlf.token", vec![b'\r', b'\n']),
        ] {
            let path = directory.path().join(name);
            let mut bytes = vec![0xff; MAX_ACCESS_TOKEN_BYTES];
            bytes.extend(suffix);
            tokio::fs::write(&path, bytes)
                .await
                .expect("write boundary token");
            let credential = read_access_credential(Some(&path))
                .await
                .expect("read boundary token");
            let bytes = credential.expose_to_policy();
            assert_eq!(bytes.len(), MAX_ACCESS_TOKEN_BYTES);
            assert!(bytes.iter().all(|byte| *byte == 0xff));
        }

        let oversized = directory.path().join("oversized.token");
        let mut oversized_bytes = vec![b'x'; MAX_ACCESS_TOKEN_BYTES + 1];
        oversized_bytes.push(b'\n');
        tokio::fs::write(&oversized, oversized_bytes)
            .await
            .expect("write oversized token");
        let error = match read_access_credential(Some(&oversized)).await {
            Ok(_) => panic!("oversized token must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "AuthenticationRequired");
    }

    #[tokio::test]
    async fn missing_or_empty_credential_is_authentication_required() {
        let error = match read_access_credential(None).await {
            Ok(_) => panic!("missing token path must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "AuthenticationRequired");

        let directory = TempDir::new().expect("temporary token directory");
        let empty = directory.path().join("empty.token");
        tokio::fs::write(&empty, [])
            .await
            .expect("write empty token");
        let error = match read_access_credential(Some(&empty)).await {
            Ok(_) => panic!("empty token must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "AuthenticationRequired");
    }
}
