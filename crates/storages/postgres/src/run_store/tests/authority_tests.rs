use super::*;

async fn mutate_store_metadata_unchecked(pool: &PgPool, statements: &[&'static str]) {
    sqlx::query("ALTER TABLE store_metadata DISABLE TRIGGER store_metadata_no_update")
        .execute(pool)
        .await
        .expect("disable metadata mutation guard");
    for statement in statements.iter().copied() {
        sqlx::query(statement)
            .execute(pool)
            .await
            .expect("mutate store metadata fixture");
    }
    sqlx::query("ALTER TABLE store_metadata ENABLE TRIGGER store_metadata_no_update")
        .execute(pool)
        .await
        .expect("reenable metadata mutation guard");
}

#[tokio::test]
async fn schema_validation_accepts_store_commit_order_authority() {
    let (store, schema) = test_store().await;
    let mut tx = store.pool.begin().await.expect("begin transaction");
    let commit_order: i64 = sqlx::query_scalar(
        "INSERT INTO commits \
         (commit_id, run_id, seq, commit_key, commit_purpose, prepared_commit_plan_fingerprint, \
          commit_batch_hash, store_commit_order, event_count) \
         VALUES \
         ('schema-trigger-commit', 'schema-trigger-run', 1, 'schema-trigger-key', \
          'schema-trigger-purpose', $1, $2, 1, 1) \
         RETURNING store_commit_order",
    )
    .bind(content_digest(252).as_str())
    .bind(content_digest(253).as_str())
    .fetch_one(&mut *tx)
    .await
    .expect("insert commit with explicit store commit order");
    assert_eq!(commit_order, 1);
    tx.rollback().await.expect("rollback manual authority rows");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect("schema validation still passes");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn store_authority_rejects_schema_drift_cases() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingMigrationRecord,
        MigrationChecksumMismatch,
        StaleSchemaObject,
        MissingFactProjectionTable,
        RetiredFactProjectionObject,
        InvalidStoreMetadata,
        InvalidStoreScopeBinding,
        MissingMutationGuardTrigger,
    }

    for (case, expected) in [
        (
            Case::MissingMigrationRecord,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::MigrationChecksumMismatch,
            PostgresStoreAuthorityError::MigrationChecksumMismatch,
        ),
        (
            Case::StaleSchemaObject,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::MissingFactProjectionTable,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::RetiredFactProjectionObject,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::InvalidStoreMetadata,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::InvalidStoreScopeBinding,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
        (
            Case::MissingMutationGuardTrigger,
            PostgresStoreAuthorityError::StoreAuthorityMismatch,
        ),
    ] {
        let (store, schema) = test_store().await;

        match case {
            Case::MissingMigrationRecord => {
                sqlx::query("DELETE FROM _sqlx_migrations")
                    .execute(&store.pool)
                    .await
                    .expect("delete migration ledger");
            }
            Case::MigrationChecksumMismatch => {
                sqlx::query(
                    "UPDATE _sqlx_migrations SET checksum = decode(repeat('00', 32), 'hex')",
                )
                .execute(&store.pool)
                .await
                .expect("mutate migration checksum");
            }
            Case::StaleSchemaObject => {
                sqlx::query("CREATE TABLE typed_run_heads (id TEXT PRIMARY KEY)")
                    .execute(&store.pool)
                    .await
                    .expect("create stale retired table");
            }
            Case::MissingFactProjectionTable => {
                sqlx::query("DROP TABLE fact_query_terms")
                    .execute(&store.pool)
                    .await
                    .expect("drop fact term table");
            }
            Case::RetiredFactProjectionObject => {
                sqlx::query("CREATE TABLE typed_fact_projection (id TEXT PRIMARY KEY)")
                    .execute(&store.pool)
                    .await
                    .expect("create retired fact projection table");
            }
            Case::InvalidStoreMetadata => {
                mutate_store_metadata_unchecked(
                    &store.pool,
                    &["UPDATE store_metadata SET schema_contract_version = 'mfm.postgres.store.unsupported'"],
                )
                .await;
            }
            Case::InvalidStoreScopeBinding => {
                mutate_store_metadata_unchecked(
                    &store.pool,
                    &[
                        "ALTER TABLE store_metadata ALTER COLUMN store_scope_id DROP NOT NULL",
                        "UPDATE store_metadata SET store_scope_id = NULL",
                    ],
                )
                .await;
            }
            Case::MissingMutationGuardTrigger => {
                sqlx::query("DROP TRIGGER run_events_no_update ON run_events")
                    .execute(&store.pool)
                    .await
                    .expect("drop run events mutation guard");
            }
        }

        let error = crate::schema::validate_pool(&store.pool)
            .await
            .expect_err("schema drift should fail authority validation");
        assert_authority_error(error, expected);

        drop_schema(&store, &schema).await;
    }
}

#[tokio::test]
async fn store_scope_survives_reconnects_and_rejects_mutation() {
    let (store, schema) = test_store().await;

    let store_scope = store.load_store_scope_id().await.expect("load store scope");
    assert!(store_scope.as_str().starts_with(StoreScopeId::PREFIX));
    assert_eq!(store.store_authority().store_scope_id(), &store_scope);

    let restarted = PostgresStore {
        pool: store.pool.clone(),
        authority: store.store_authority().clone(),
        committed_journal_load_test_barrier: None,
    };
    let restarted_store_scope = restarted
        .load_store_scope_id()
        .await
        .expect("load restarted store scope");
    assert_eq!(store_scope, restarted_store_scope);

    sqlx::query(
        "UPDATE store_metadata \
         SET store_scope_id = 'mfm.store_scope.v1:ffffffffffffffffffffffffffffffff' \
         WHERE singleton",
    )
    .execute(&store.pool)
    .await
    .expect_err("store scope mutation is rejected");
    crate::schema::validate_pool(&store.pool)
        .await
        .expect("metadata remains valid");

    drop_schema(&store, &schema).await;
}

fn assert_authority_error(error: PostgresStoreError, expected: PostgresStoreAuthorityError) {
    let PostgresStoreError::Authority(actual) = error else {
        panic!("expected store authority error, got {error:?}");
    };
    assert_eq!(actual, expected);
}
