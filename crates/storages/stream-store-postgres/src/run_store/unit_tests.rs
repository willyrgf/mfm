use super::*;

#[test]
fn artifact_role_contract_postgres_tag_roundtrip_uses_events_contract() {
    for role in events::ArtifactRole::ALL {
        assert_eq!(
            decode_artifact_role_tag(role.as_str()).expect("role tag parses"),
            *role
        );
    }

    assert!(matches!(
        decode_artifact_role_tag("unknown_artifact_role"),
        Err(PostgresStoreError::Store(StoreError::Identity(message)))
            if message.contains("unknown artifact role unknown_artifact_role")
    ));
}
