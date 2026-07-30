use std::fs::File;
use std::io::Read;

use mfm_canonical::{CanonicalValue, RecoverabilityContractV3, ValidatedCanonicalValueV3};
use mfm_ids::{ContentRef, DigestBytes};

use crate::{ErrorClass, PublicError};

const EXECUTABLE_DESCRIPTOR_CONTRACT: &str = "mfm.executable-bytes-descriptor.v1";
const EXECUTABLE_BYTES_CONTRACT: &str = "mfm.executable-bytes.v1";
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Self-attested identity and exact retained descriptor of the serving executable.
pub(super) struct CurrentExecutableIdentity {
    descriptor: ValidatedCanonicalValueV3,
    content_ref: ContentRef,
}

impl std::fmt::Debug for CurrentExecutableIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CurrentExecutableIdentity")
            .field("content_ref", &self.content_ref)
            .finish_non_exhaustive()
    }
}

impl CurrentExecutableIdentity {
    pub(super) const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    pub(super) fn descriptor_bytes(&self) -> &[u8] {
        self.descriptor.as_bytes()
    }
}

pub(super) async fn current_executable_identity() -> Result<CurrentExecutableIdentity, PublicError>
{
    resolve_executable_identity_with(read_current_executable_sha256).await
}

async fn resolve_executable_identity_with<F>(
    read: F,
) -> Result<CurrentExecutableIdentity, PublicError>
where
    F: FnOnce() -> Result<DigestBytes, ()> + Send + 'static,
{
    let raw_sha256 = tokio::task::spawn_blocking(read)
        .await
        .map_err(|_| executable_identity_unavailable())?
        .map_err(|_| executable_identity_unavailable())?;
    executable_identity(&raw_sha256)
}

fn executable_identity(raw_sha256: &DigestBytes) -> Result<CurrentExecutableIdentity, PublicError> {
    let value = CanonicalValue::object([
        (
            "contract",
            CanonicalValue::String(EXECUTABLE_BYTES_CONTRACT.to_owned()),
        ),
        ("sha256", CanonicalValue::String(raw_sha256.to_string())),
    ])
    .map_err(|_| executable_identity_unavailable())?;
    let contract =
        RecoverabilityContractV3::embedded().map_err(|_| executable_identity_unavailable())?;
    let descriptor = contract
        .encode(EXECUTABLE_DESCRIPTOR_CONTRACT, &value)
        .map_err(|_| executable_identity_unavailable())?;
    let content_ref = contract
        .content_ref(&descriptor)
        .map_err(|_| executable_identity_unavailable())?;
    Ok(CurrentExecutableIdentity {
        descriptor,
        content_ref,
    })
}

fn read_current_executable_sha256() -> Result<DigestBytes, ()> {
    read_stable_current_executable_sha256()
}

#[cfg(target_os = "linux")]
fn read_stable_current_executable_sha256() -> Result<DigestBytes, ()> {
    let mut file = File::open("/proc/self/exe").map_err(|_| ())?;
    let before = file_identity(&file.metadata().map_err(|_| ())?);
    let digest = streaming_sha256(&mut file)?;
    let after = file_identity(&file.metadata().map_err(|_| ())?);
    if before != after {
        return Err(());
    }
    Ok(digest)
}

#[cfg(target_os = "macos")]
fn read_stable_current_executable_sha256() -> Result<DigestBytes, ()> {
    let path = std::env::current_exe().map_err(|_| ())?;
    let mut file = File::open(&path).map_err(|_| ())?;
    let file_before = file_identity(&file.metadata().map_err(|_| ())?);
    let path_before = file_identity(&std::fs::metadata(&path).map_err(|_| ())?);
    if file_before != path_before {
        return Err(());
    }
    let digest = streaming_sha256(&mut file)?;
    let file_after = file_identity(&file.metadata().map_err(|_| ())?);
    let path_after = file_identity(&std::fs::metadata(path).map_err(|_| ())?);
    if file_before != file_after || file_before != path_after {
        return Err(());
    }
    Ok(digest)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_stable_current_executable_sha256() -> Result<DigestBytes, ()> {
    Err(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn file_identity(metadata: &std::fs::Metadata) -> (u64, u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    (metadata.dev(), metadata.ino(), metadata.size())
}

fn streaming_sha256(reader: &mut dyn Read) -> Result<DigestBytes, ()> {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    let digest = context.finish();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(digest.as_ref());
    Ok(DigestBytes::from_array(bytes))
}

pub(super) fn executable_identity_unavailable() -> PublicError {
    PublicError::backend(
        ErrorClass::ServiceUnavailable,
        "ExecutableIdentityUnavailable",
        "The running executable could not be identified",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_sha256_matches_one_shot_for_boundary_lengths() {
        let fixtures = [
            Vec::new(),
            b"mfm executable identity".to_vec(),
            (0..(HASH_BUFFER_BYTES * 3 + 17))
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>(),
        ];
        for fixture in fixtures {
            let actual = streaming_sha256(&mut std::io::Cursor::new(&fixture)).expect("hash");
            assert_eq!(actual, mfm_canonical::sha256_digest_bytes(&fixture));
        }
    }

    #[test]
    fn executable_identity_uses_the_frozen_annex_descriptor() {
        let raw_sha256 = DigestBytes::from_array([0_u8; 32]);
        let identity = executable_identity(&raw_sha256).expect("identity");
        assert_eq!(
            identity.descriptor_bytes(),
            br#"{"contract":"mfm.executable-bytes.v1","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#
        );
        assert_eq!(
            identity.content_ref().schema_id().as_str(),
            "schema:mfm.executable-bytes-descriptor:1:sha256-jcs-v1:40bca04a66ae22de3349f76e1e60822429609a7aad400ecbf65489836478ce66"
        );
    }

    #[tokio::test]
    async fn current_identity_binds_the_running_test_executable_bytes() {
        let identity = current_executable_identity()
            .await
            .expect("current executable identity");
        let bytes = std::fs::read(std::env::current_exe().expect("current executable"))
            .expect("read current executable");
        let expected = executable_identity(&mfm_canonical::sha256_digest_bytes(&bytes))
            .expect("canonical executable identity");
        assert_eq!(identity.content_ref(), expected.content_ref());
        assert_eq!(identity.descriptor_bytes(), expected.descriptor_bytes());
    }

    #[tokio::test]
    async fn executable_identity_failures_share_one_redacted_error() {
        let read_error = resolve_executable_identity_with(|| Err(()))
            .await
            .expect_err("read failure");
        let join_error = resolve_executable_identity_with(|| panic!())
            .await
            .expect_err("join failure");
        assert_eq!(read_error, executable_identity_unavailable());
        assert_eq!(join_error, executable_identity_unavailable());
        let rendered = serde_json::to_string(&read_error).expect("public error JSON");
        assert!(!rendered.contains("/proc"));
        assert!(!rendered.contains("JoinError"));
    }
}
