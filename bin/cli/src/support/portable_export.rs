use std::path::Path;

use mfm_app::{PortableExportInput, PublicError};
use mfm_ids::ContentRef;
use tokio::io::AsyncReadExt;

const MAX_PORTABLE_EXPORT_REF_BYTES: usize = 4_096;

/// Reads one caller-held canonical semantic export and its exact canonical `ContentRef` sidecar.
pub(crate) async fn read_portable_export_input(
    export_path: &Path,
    content_ref_path: &Path,
) -> Result<PortableExportInput, PublicError> {
    let content_ref_bytes = read_bounded(content_ref_path, MAX_PORTABLE_EXPORT_REF_BYTES).await?;
    let content_ref: ContentRef =
        serde_json::from_slice(&content_ref_bytes).map_err(|_| invalid_input())?;
    let canonical_ref = serde_json::to_vec(&content_ref).map_err(|_| invalid_input())?;
    if canonical_ref != content_ref_bytes {
        return Err(invalid_input());
    }

    let export_bytes = read_bounded(export_path, mfm_app::MAX_REPLAY_PORTABLE_EXPORT_BYTES).await?;
    PortableExportInput::from_bytes(content_ref, export_bytes)
}

async fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, PublicError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| invalid_input())?;
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| invalid_input())?;
    if bytes.is_empty() {
        return Err(invalid_input());
    }
    if bytes.len() > max_bytes {
        return Err(PublicError::replay_artifact_too_large());
    }
    Ok(bytes)
}

fn invalid_input() -> PublicError {
    PublicError::replay_artifact_invalid()
}

#[cfg(test)]
mod tests {
    use mfm_ids::{ContentDigest, ContentRef, SchemaId};
    use tempfile::TempDir;

    use super::{read_bounded, read_portable_export_input, MAX_PORTABLE_EXPORT_REF_BYTES};

    #[tokio::test]
    async fn reader_requires_exact_canonical_content_ref_bytes() {
        let directory = TempDir::new().expect("temporary export directory");
        let export_path = directory.path().join("run.export");
        let ref_path = directory.path().join("run.export.ref");
        let bytes = br#"{"portable":"evidence"}"#;
        tokio::fs::write(&export_path, bytes)
            .await
            .expect("portable export");
        let content_ref = ContentRef::new(
            SchemaId::parse(
                "schema:mfm.portable-run-export:1:sha256-jcs-v1:\
                 3a270e7b87eab6cc696813f2e947aed6503f3744be51709059bee23f4934c82d",
            )
            .expect("portable schema"),
            ContentDigest::parse(
                "content:sha256-v1:\
                 120b15311bb4011d6d7dd26a9311d2abfd12b8be74275d2031bf0f49c44304ad",
            )
            .expect("portable digest"),
        )
        .expect("content ref");
        let canonical = serde_json::to_vec(&content_ref).expect("canonical content ref");
        tokio::fs::write(&ref_path, &canonical)
            .await
            .expect("content ref sidecar");

        let input = read_portable_export_input(&export_path, &ref_path)
            .await
            .expect("portable input");
        assert_eq!(input.content_ref(), &content_ref);
        assert_eq!(input.bytes(), bytes);

        let mut noncanonical = canonical;
        noncanonical.push(b'\n');
        tokio::fs::write(&ref_path, noncanonical)
            .await
            .expect("noncanonical ref sidecar");
        let error = read_portable_export_input(&export_path, &ref_path)
            .await
            .expect_err("noncanonical ref sidecar");
        assert_eq!(error.code, "ReplayArtifactInvalid");

        for invalid in [
            serde_json::json!({
                "content_digest": content_ref.content_digest(),
                "schema_id": content_ref.schema_id(),
                "unknown": true,
            })
            .to_string(),
            format!(
                r#"{{"content_digest":"{}","content_digest":"{}","schema_id":"{}"}}"#,
                content_ref.content_digest(),
                content_ref.content_digest(),
                content_ref.schema_id(),
            ),
        ] {
            tokio::fs::write(&ref_path, invalid)
                .await
                .expect("invalid ref sidecar");
            let error = read_portable_export_input(&export_path, &ref_path)
                .await
                .expect_err("invalid ref sidecar");
            assert_eq!(error.code, "ReplayArtifactInvalid");
        }

        tokio::fs::write(&ref_path, vec![b'x'; MAX_PORTABLE_EXPORT_REF_BYTES + 1])
            .await
            .expect("oversized ref sidecar");
        let error = read_portable_export_input(&export_path, &ref_path)
            .await
            .expect_err("oversized ref sidecar");
        assert_eq!(error.code, "ReplayArtifactTooLarge");
    }

    #[tokio::test]
    async fn bounded_reader_rejects_empty_and_limit_plus_one() {
        assert_eq!(MAX_PORTABLE_EXPORT_REF_BYTES, 4_096);
        let directory = TempDir::new().expect("temporary export directory");
        let path = directory.path().join("bounded.input");

        tokio::fs::write(&path, b"")
            .await
            .expect("empty bounded input");
        let error = read_bounded(&path, 4)
            .await
            .expect_err("empty bounded input");
        assert_eq!(error.code, "ReplayArtifactInvalid");

        tokio::fs::write(&path, b"1234")
            .await
            .expect("boundary input");
        assert_eq!(
            read_bounded(&path, 4).await.expect("bounded input"),
            b"1234"
        );

        tokio::fs::write(&path, b"12345")
            .await
            .expect("oversized input");
        let error = read_bounded(&path, 4).await.expect_err("oversized input");
        assert_eq!(error.code, "ReplayArtifactTooLarge");
    }
}
