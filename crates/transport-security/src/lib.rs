#![warn(missing_docs)]
//! Exact checked TLS-root authority shared by live transports.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use rustls::RootCertStore;
use serde::{Deserialize, Deserializer};
use tokio::io::AsyncReadExt;

/// Maximum encoded bytes accepted in one PEM root bundle.
pub const MAX_PEM_ROOT_BYTES: usize = 256 * 1024;
/// Maximum encoded bytes accepted in one absolute UTF-8 PEM path.
pub const MAX_PEM_PATH_BYTES: usize = 4096;

/// Redaction-safe TLS-root specification or loading failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("TLS root authority is invalid or unavailable")]
pub struct TlsRootError;

/// Checked absolute UTF-8 path to a content-pinned PEM root bundle.
///
/// This value deliberately implements neither `Debug` nor `Display` so a
/// private deployment path cannot enter diagnostics accidentally.
#[derive(Clone, PartialEq, Eq)]
pub struct PemRootPath(PathBuf);

impl PemRootPath {
    /// Checks one absolute UTF-8 path without accessing the filesystem.
    pub fn new(value: impl AsRef<str>) -> Result<Self, TlsRootError> {
        let value = value.as_ref();
        if value.is_empty()
            || value.len() > MAX_PEM_PATH_BYTES
            || value.as_bytes().contains(&0)
            || !Path::new(value).is_absolute()
        {
            return Err(TlsRootError);
        }
        Ok(Self(PathBuf::from(value)))
    }

    fn as_path(&self) -> &Path {
        &self.0
    }
}

/// One exhaustive checked source of server-authentication roots.
///
/// The specification implements neither `Debug`, `Display`, nor `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub enum TlsRootSpec {
    /// The exact compiled WebPKI root set.
    WebPki,
    /// One exclusive content-pinned PEM root bundle.
    PemFile {
        /// Checked private filesystem path.
        path: PemRootPath,
        /// Exact retained-byte digest, fixed to `sha256-v1`.
        digest: ContentDigest,
    },
}

impl<'de> Deserialize<'de> for TlsRootSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: String,
            path: Option<String>,
            digest: Option<ContentDigest>,
        }

        match Wire::deserialize(deserializer)? {
            Wire {
                kind,
                path: None,
                digest: None,
            } if kind == "webpki" => Ok(Self::WebPki),
            Wire {
                kind,
                path: Some(path),
                digest: Some(digest),
            } if kind == "pem-file" => {
                if digest.algorithm() != DigestAlgorithm::Sha256V1 {
                    return Err(serde::de::Error::custom(TlsRootError));
                }
                Ok(Self::PemFile {
                    path: PemRootPath::new(path).map_err(serde::de::Error::custom)?,
                    digest,
                })
            }
            _ => Err(serde::de::Error::custom(TlsRootError)),
        }
    }
}

/// One exact immutable root store ready for a protocol client.
///
/// This value deliberately exposes no certificate or path rendering.
#[derive(Clone)]
pub struct LoadedTlsRoots {
    roots: Arc<RootCertStore>,
    source: LoadedRootSource,
}

#[derive(Clone, Copy)]
enum LoadedRootSource {
    WebPki,
    PemFile,
}

impl LoadedTlsRoots {
    /// Loads and verifies one root specification.
    pub async fn load(spec: &TlsRootSpec) -> Result<Self, TlsRootError> {
        match spec {
            TlsRootSpec::WebPki => Ok(Self {
                roots: Arc::new(RootCertStore::from_iter(
                    webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                )),
                source: LoadedRootSource::WebPki,
            }),
            TlsRootSpec::PemFile { path, digest } => {
                let bytes = read_pinned_regular_file(path, digest).await?;
                let mut roots = RootCertStore::empty();
                let mut count = 0usize;
                for certificate in CertificateDer::pem_slice_iter(&bytes) {
                    roots
                        .add(certificate.map_err(|_| TlsRootError)?)
                        .map_err(|_| TlsRootError)?;
                    count = count.checked_add(1).ok_or(TlsRootError)?;
                }
                if count == 0 {
                    return Err(TlsRootError);
                }
                Ok(Self {
                    roots: Arc::new(roots),
                    source: LoadedRootSource::PemFile,
                })
            }
        }
    }

    /// Returns the exact immutable Rustls root store.
    pub fn root_store(&self) -> Arc<RootCertStore> {
        Arc::clone(&self.roots)
    }

    /// Returns whether the roots are the exact compiled WebPKI set.
    pub const fn is_webpki(&self) -> bool {
        matches!(self.source, LoadedRootSource::WebPki)
    }

    /// Returns the number of retained trust anchors.
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// Returns whether no trust anchor was retained.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

async fn read_pinned_regular_file(
    path: &PemRootPath,
    expected: &ContentDigest,
) -> Result<Vec<u8>, TlsRootError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let file = options
        .open(path.as_path())
        .await
        .map_err(|_| TlsRootError)?;
    let metadata = file.metadata().await.map_err(|_| TlsRootError)?;
    if !metadata.file_type().is_file() {
        return Err(TlsRootError);
    }
    let mut bytes = Vec::new();
    file.take((MAX_PEM_ROOT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| TlsRootError)?;
    if bytes.is_empty()
        || bytes.len() > MAX_PEM_ROOT_BYTES
        || &raw_content_digest(&bytes) != expected
    {
        return Err(TlsRootError);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_spec_wire_is_exhaustive_and_redaction_safe() {
        let webpki: TlsRootSpec = serde_json::from_str(r#"{"kind":"webpki"}"#).expect("webpki");
        assert!(matches!(webpki, TlsRootSpec::WebPki));
        for rejected in [
            r#"{"kind":"webpki","path":"/tmp/ca.pem"}"#,
            r#"{"kind":"native"}"#,
            r#"{"kind":"pem-file","path":"relative.pem","digest":"content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#,
            r#"{"kind":"pem-file","path":"/tmp/ca.pem","digest":"content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#,
        ] {
            assert!(
                serde_json::from_str::<TlsRootSpec>(rejected).is_err(),
                "accepted {rejected}"
            );
        }
        assert_eq!(
            TlsRootError.to_string(),
            "TLS root authority is invalid or unavailable"
        );
    }

    #[tokio::test]
    async fn webpki_is_nonempty_and_exactly_classified() {
        let loaded = LoadedTlsRoots::load(&TlsRootSpec::WebPki)
            .await
            .expect("compiled roots");
        assert!(loaded.is_webpki());
        assert!(!loaded.is_empty());
        assert_eq!(loaded.len(), webpki_roots::TLS_SERVER_ROOTS.len());
    }
}
