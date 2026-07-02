use super::*;

#[test]
fn artifact_role_contract_postgres_tag_roundtrip_uses_events_contract() {
    assert_eq!(
        decode_artifact_role_tag("fact_descriptor").expect("fact descriptor role tag parses"),
        events::ArtifactRole::FactDescriptor
    );

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

#[tokio::test]
async fn store_authority_connection_error_redacts_database_url() {
    let database_url = "postgres://mfm_user:super-secret@127.0.0.1:notaport/mfm";
    let error = match PostgresRunStore::connect(database_url).await {
        Ok(_) => panic!("invalid postgres URL should not connect"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        PostgresStoreError::Authority(PostgresStoreAuthorityError::Connection)
    ));
    let rendered = format!("{error:?}\n{error}");
    for forbidden in [
        database_url,
        "mfm_user",
        "super-secret",
        "127.0.0.1",
        "notaport",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "store authority error leaked `{forbidden}` in {rendered}"
        );
    }
}
