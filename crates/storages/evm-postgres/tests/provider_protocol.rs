//! Hostile-wire qualification for the separate wallet authority provider.

use std::sync::atomic::{AtomicU64, Ordering};

use mfm_canonical::sha256_digest_bytes;
use mfm_evm::EvmWalletReference;
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use mfm_storage_evm_postgres::{
    FinishedDeploymentAssembly, PendingDeploymentAssembly, PostgresEvmWalletError,
    QualifiedEvmRoutingCatalog, WalletAuthorityProviderClient, WalletAuthorityProviderTrust,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn deployment_assembly_bearers_are_affine_and_nonserializable() {
    static_assertions::assert_not_impl_any!(QualifiedEvmRoutingCatalog: Clone, serde::Serialize);
    static_assertions::assert_not_impl_any!(PendingDeploymentAssembly: Clone, serde::Serialize);
    static_assertions::assert_not_impl_any!(FinishedDeploymentAssembly: Clone, serde::Serialize);
}

#[tokio::test]
async fn hostile_provider_replies_fail_closed_without_leaking_wire_text() {
    const PROVIDER_TEXT: &str = "hostile-provider-text-must-not-escape";

    let cases = [
        (
            serde_json::json!({
                "kind": "authenticated",
                "signature": "00".repeat(64),
                "provider_text": PROVIDER_TEXT,
            })
            .to_string(),
            PostgresEvmWalletError::InvalidAuthority,
        ),
        (
            serde_json::json!({
                "kind": "authenticated",
                "signature": PROVIDER_TEXT,
            })
            .to_string(),
            PostgresEvmWalletError::FenceRejected,
        ),
    ];

    for (reply, expected) in cases {
        let socket_path = std::env::temp_dir().join(format!(
            "mfm-wallet-provider-hostile-{}-{}.sock",
            std::process::id(),
            SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let listener = UnixListener::bind(&socket_path).expect("bind hostile provider socket");
        let provider = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept provider client");
            let (read, mut write) = tokio::io::split(stream);
            let mut read = BufReader::new(read);
            let mut request = String::new();
            read.read_line(&mut request)
                .await
                .expect("read authentication request");
            let request: serde_json::Value =
                serde_json::from_str(&request).expect("authentication request");
            assert_eq!(request["version"], 4);
            assert!(request.get("current_chain_registry_head_ref").is_some());
            assert!(request.get("minimum_chain_registry_head_ref").is_none());
            write
                .write_all(reply.as_bytes())
                .await
                .expect("write hostile provider reply");
            write
                .write_all(b"\n")
                .await
                .expect("terminate hostile provider reply");
        });

        let trust = WalletAuthorityProviderTrust::new(
            StableId::new("mfm.evm.test/hostile-provider").expect("provider identity"),
            authority_reference(0x91),
            authority_reference(0x92),
            authority_reference(0x93),
            &"11".repeat(32),
        )
        .expect("provider trust anchor");
        let error = WalletAuthorityProviderClient::connect_unix(&socket_path, trust)
            .await
            .expect_err("hostile provider reply must fail");
        assert_eq!(error, expected);
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(PROVIDER_TEXT));
        assert!(!rendered.contains(&socket_path.to_string_lossy().into_owned()));

        provider.await.expect("hostile provider task");
        std::fs::remove_file(&socket_path).expect("remove hostile provider socket");
    }
}

#[test]
fn trust_rejects_cross_role_authority_reference_reuse() {
    let shared = authority_reference(0xa1);
    assert!(WalletAuthorityProviderTrust::new(
        StableId::new("mfm.evm.test/reused-provider-reference").expect("provider identity"),
        shared.clone(),
        shared,
        authority_reference(0xa2),
        &"11".repeat(32),
    )
    .is_err());
}

fn authority_reference(discriminator: u8) -> EvmWalletReference {
    let schema_id = SchemaId::new(
        "mfm.test.provider-authority-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.provider-authority-reference.v1"),
    )
    .expect("authority reference schema");
    EvmWalletReference::from_content_ref(
        ContentRef::new(
            schema_id,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator]),
            ),
        )
        .expect("authority content reference"),
    )
}
