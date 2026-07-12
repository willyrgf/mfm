use super::*;

#[test]
pub(super) fn mfm_error_info_constructor_accepts_redacted_diagnostic_ref() {
    let diagnostic = event_artifact_ref(
        artifact_id(42),
        ArtifactRole::RedactedDiagnostic,
        schema_id("mfm.test.diagnostic", 43),
        content_digest(44),
    );

    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "runtime validation failed",
    )
    .expect("base error")
    .with_public_details(RedactedJson::new(content_digest(45)))
    .expect("public details")
    .with_diagnostic_ref(diagnostic.clone())
    .expect("diagnostic ref");

    assert_eq!(error.diagnostic_ref, Some(diagnostic));
    assert!(error.public_details.is_some());
}

#[test]
pub(super) fn mfm_error_info_rejects_public_diagnostic_boundary_violations() {
    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "provider returned bearer token=super-secret-value",
    )
    .expect_err("secret-shaped message rejected");

    assert!(matches!(
        error,
        EventError::InvalidPublicDiagnostic {
            field: "safe_message",
            reason: "message resembles secret material"
        }
    ));

    let diagnostic = event_artifact_ref(
        artifact_id(46),
        ArtifactRole::SideEffectIntent,
        schema_id("mfm.test.diagnostic", 47),
        content_digest(48),
    );
    let error = MfmErrorInfo::new(
        ErrorCode::new("redacted_diagnostic").expect("code"),
        ErrorCategory::Runtime,
        false,
        "runtime validation failed",
    )
    .expect("base error")
    .with_diagnostic_ref(diagnostic)
    .expect_err("wrong diagnostic role rejected");

    assert!(matches!(
        error,
        EventError::InvalidPublicDiagnostic {
            field: "diagnostic_ref",
            reason: "diagnostic artifact role must be redacted_diagnostic"
        }
    ));
}

pub(super) fn error_with_diagnostic(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> MfmErrorInfo {
    MfmErrorInfo {
        code: ErrorCode::new("event_accessor_test").expect("error code"),
        category: ErrorCategory::Runtime,
        retryable: false,
        safe_message: "event accessor test".to_owned(),
        public_details: None,
        diagnostic_ref: Some(event_artifact_ref(
            artifact_id,
            role,
            schema_id,
            content_digest,
        )),
    }
}

pub(super) fn touched_set(byte: u8) -> ResourceTouchedSetEvidence {
    ResourceTouchedSetEvidence {
        namespace: ResourceNamespace::new("mfm.test.resource").expect("namespace"),
        evidence_schema_id: schema_id("mfm.test.touched_set", byte),
        evidence_hash: content_digest(byte.wrapping_add(1)),
        evidence_artifact_id: artifact_id(byte.wrapping_add(2)),
        evidence_artifact_evidence_hash: content_digest(byte.wrapping_add(3)),
    }
}

pub(super) fn store_scope_id() -> StoreScopeId {
    StoreScopeId::new("mfm.store_scope.v1:20202020202020202020202020202020").expect("store scope")
}

pub(super) fn run_identity_material(spec_hash: SpecHash) -> RunIdentityMaterialV1 {
    RunIdentityMaterialV1 {
        certified_spec_hash: spec_hash,
        store_scope_id: store_scope_id(),
        invocation_key_digest: content_digest(1),
    }
}

#[test]
pub(super) fn invocation_key_material_v1_digest_is_canonical_and_redacted() {
    let material = InvocationKeyMaterialV1::new("alpha").expect("invocation key");

    assert_eq!(
        material.canonical_json().expect("canonical").as_str(),
        r#"{"domain":"mfm.invocation_key.v1","raw_key":"alpha"}"#
    );
    assert_eq!(
        material.digest().expect("digest").as_str(),
        "content:sha256-jcs-v1:e8b2dba1a1730580547875fc9f27a3edf6a5c62660bdc3a6ac06518b487ecd43"
    );
    assert!(!format!("{material:?}").contains("alpha"));
}

#[test]
pub(super) fn invocation_key_material_v1_rejects_empty_and_oversized_keys_without_echo() {
    let empty = InvocationKeyMaterialV1::new("").expect_err("empty key rejects");
    assert_eq!(empty.to_string(), "invalid invocation key: empty");

    let too_large = "x".repeat(InvocationKeyMaterialV1::MAX_RAW_KEY_BYTES + 1);
    let err = InvocationKeyMaterialV1::new(&too_large).expect_err("oversized key rejects");
    assert_eq!(err.to_string(), "invalid invocation key: too_large");
    assert!(!err.to_string().contains(&too_large));
}
