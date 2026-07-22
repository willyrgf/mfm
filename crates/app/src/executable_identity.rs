use std::fs::File;
use std::io::Read;

use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{ContentDigest, DigestBytes};
use mfm_runtime::ExecutableIdentityTemplate;

use crate::{ErrorClass, PublicError};

const EXECUTABLE_IDENTITY_CONTRACT: &str = "mfm.executable-bytes.v1";
const HASH_BUFFER_BYTES: usize = 64 * 1024;

pub(super) async fn current_executable_identity_template(
) -> Result<ExecutableIdentityTemplate, PublicError> {
    resolve_executable_identity_with(read_current_executable_identity).await
}

async fn resolve_executable_identity_with<F>(
    read: F,
) -> Result<ExecutableIdentityTemplate, PublicError>
where
    F: FnOnce() -> Result<ContentDigest, ()> + Send + 'static,
{
    let digest = tokio::task::spawn_blocking(read)
        .await
        .map_err(|_| executable_identity_unavailable())?
        .map_err(|_| executable_identity_unavailable())?;
    Ok(ExecutableIdentityTemplate::new(digest))
}

fn read_current_executable_identity() -> Result<ContentDigest, ()> {
    let raw_sha256 = read_stable_current_executable_sha256()?;
    executable_identity_digest(&raw_sha256)
}

#[cfg(target_os = "linux")]
fn read_stable_current_executable_sha256() -> Result<String, ()> {
    let mut file = File::open("/proc/self/exe").map_err(|_| ())?;
    let before = file_identity(&file.metadata().map_err(|_| ())?);
    let digest = streaming_sha256(&mut file)?;
    let after = file_identity(&file.metadata().map_err(|_| ())?);
    if before != after {
        return Err(());
    }
    Ok(digest.to_string())
}

#[cfg(target_os = "macos")]
fn read_stable_current_executable_sha256() -> Result<String, ()> {
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
    Ok(digest.to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_stable_current_executable_sha256() -> Result<String, ()> {
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

fn executable_identity_digest(raw_sha256: &str) -> Result<ContentDigest, ()> {
    let identity = CanonicalValue::object([
        (
            "contract",
            CanonicalValue::String(EXECUTABLE_IDENTITY_CONTRACT.to_owned()),
        ),
        ("sha256", CanonicalValue::String(raw_sha256.to_owned())),
    ])
    .map_err(|_| ())?;
    Ok(CanonicalJsonBytes::from_value(&identity).content_digest())
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
    fn executable_identity_uses_the_exact_canonical_object() {
        let raw_sha256 = "00".repeat(32);
        let expected_json =
            format!(r#"{{"contract":"mfm.executable-bytes.v1","sha256":"{raw_sha256}"}}"#);
        let expected = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&expected_json)
            .expect("canonical fixture")
            .content_digest();
        assert_eq!(
            executable_identity_digest(&raw_sha256).expect("identity"),
            expected
        );
    }

    #[tokio::test]
    async fn current_template_binds_the_running_test_executable_bytes() {
        let template = current_executable_identity_template()
            .await
            .expect("current executable identity");
        let bytes = std::fs::read(std::env::current_exe().expect("current executable"))
            .expect("read current executable");
        let expected =
            executable_identity_digest(&mfm_canonical::sha256_digest_bytes(&bytes).to_string())
                .expect("canonical executable identity");
        assert_eq!(template.binary_digest(), &expected);
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
