use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mfm_canonical::sha256_digest_bytes;
use mfm_storage_postgres::{open_authoritative, TestAuthoritativeWriterFence};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1;

use recoverability_v1::{
    array, assert_lower_layer_owner_vector, for_each_vector, run_consumer, string, CorpusVector,
    OwnerVector,
};

const CONSUMER: &str = "mfm-storage-postgres";
const TOTAL_VECTOR_COUNT: usize = 570;
const OWNER_VECTOR_COUNT: usize = 117;
static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[tokio::test]
async fn postgres_executes_and_round_trips_the_complete_frozen_corpus() {
    let database = CorpusDatabase::create().await;
    let (qualified, _issuer) =
        open_authoritative(database.pool.clone(), TestAuthoritativeWriterFence)
            .await
            .expect("the exact corpus store schema must qualify");

    let mut owner_ids = BTreeSet::new();
    run_consumer(CONSUMER, |owner| {
        assert_lower_layer_owner_vector(owner);
        assert_postgres_owner_vector(owner);
        assert!(
            owner_ids.insert(owner.id().to_owned()),
            "duplicate PostgreSQL owner-vector execution: {}",
            owner.id()
        );
    });
    assert_eq!(owner_ids.len(), OWNER_VECTOR_COUNT);

    let mut vectors = Vec::with_capacity(TOTAL_VECTOR_COUNT);
    let mut vector_ids = BTreeSet::new();
    for_each_vector(|vector| {
        assert_postgres_coverage(vector);
        let id = vector.id().to_owned();
        assert!(
            vector_ids.insert(id.clone()),
            "duplicate frozen corpus vector id: {id}"
        );
        let bytes = vector.canonical_bytes();
        let content_digest = format!("content:sha256-v1:{}", sha256_digest_bytes(&bytes));
        vectors.push((id, content_digest, bytes));
    });
    assert_eq!(vectors.len(), TOTAL_VECTOR_COUNT);
    assert_eq!(vector_ids.len(), TOTAL_VECTOR_COUNT);

    round_trip_all_vectors(&database.pool, &vectors).await;
    assert_eq!(
        qualified.store_scope_id().as_str(),
        database.validated_store_scope_id().await,
        "physical corpus writes must remain in the qualified store scope"
    );

    drop(qualified);
    database.cleanup().await;
}

fn assert_postgres_coverage(vector: CorpusVector<'_>) {
    let covered = array(vector.vector(), "consumer_coverage")
        .iter()
        .any(|consumer| consumer.as_str() == Some(CONSUMER));
    assert!(covered, "{} omits PostgreSQL coverage", vector.id());
}

fn assert_postgres_owner_vector(owner: OwnerVector<'_>) {
    let vector = owner.vector();
    assert!(
        array(vector, "consumer_coverage")
            .iter()
            .any(|consumer| consumer.as_str() == Some(CONSUMER)),
        "{} omits PostgreSQL owner coverage",
        owner.id()
    );
    match owner {
        OwnerVector::RelationalRejection(_) => {
            assert_eq!(
                string(vector, "error_class"),
                "relational",
                "{}",
                owner.id()
            );
            assert_eq!(string(vector, "kind"), "rejection", "{}", owner.id());
            assert!(
                !string(vector, "expected_error").is_empty(),
                "{}",
                owner.id()
            );
            assert!(!string(vector, "rule").is_empty(), "{}", owner.id());
            assert!(!string(vector, "target").is_empty(), "{}", owner.id());
        }
        OwnerVector::RelationalPositive(_) => {
            assert!(
                matches!(
                    owner.kind(),
                    "commit_coordinate_separation"
                        | "content_identity_separation"
                        | "export_identity"
                        | "frontier_order"
                        | "identity_separation"
                        | "read_returned_validation_verdict"
                        | "read_safe_failure_verdict"
                        | "relational_acceptance"
                        | "request_identity"
                        | "resource_refold"
                        | "schema_identity"
                        | "type_separation"
                ),
                "{} has no PostgreSQL owner assertion",
                owner.id()
            );
        }
    }
}

async fn round_trip_all_vectors(pool: &PgPool, vectors: &[(String, String, Vec<u8>)]) {
    let mut transaction = pool.begin().await.expect("begin corpus write");
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .expect("use the migration-granted application role");
    for (id, content_digest, bytes) in vectors {
        let byte_length = i64::try_from(bytes.len()).expect("bounded corpus vector");
        let result = sqlx::query(
            "INSERT INTO artifact_blobs (content_digest, byte_length, bytes) \
             VALUES ($1, $2, $3)",
        )
        .bind(content_digest)
        .bind(byte_length)
        .bind(bytes)
        .execute(&mut *transaction)
        .await
        .unwrap_or_else(|error| panic!("{id}: PostgreSQL rejected physical vector: {error}"));
        assert_eq!(result.rows_affected(), 1, "{id}");
    }
    transaction.commit().await.expect("commit all corpus blobs");

    let mut transaction = pool.begin().await.expect("begin corpus read");
    sqlx::query("SET LOCAL ROLE mfm_store_application")
        .execute(&mut *transaction)
        .await
        .expect("use the application role for physical reads");
    let rows = sqlx::query(
        "SELECT content_digest, byte_length::text AS byte_length, bytes \
           FROM artifact_blobs \
          ORDER BY content_digest",
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("load every physical corpus vector");
    let admission_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM artifact_admissions")
        .fetch_one(&mut *transaction)
        .await
        .expect("count artifact authority");
    transaction.commit().await.expect("commit corpus read");

    assert_eq!(rows.len(), TOTAL_VECTOR_COUNT);
    assert_eq!(
        admission_count, 0,
        "staged physical bytes must not mint artifact authority"
    );

    let expected = vectors
        .iter()
        .map(|(id, digest, bytes)| (digest.as_str(), (id.as_str(), bytes.as_slice())))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(expected.len(), TOTAL_VECTOR_COUNT);
    for row in rows {
        let digest: String = row.try_get("content_digest").expect("content digest");
        let byte_length: String = row.try_get("byte_length").expect("byte length");
        let bytes: Vec<u8> = row.try_get("bytes").expect("blob bytes");
        let (id, exact_bytes) = expected
            .get(digest.as_str())
            .unwrap_or_else(|| panic!("unexpected physical content digest {digest}"));
        assert_eq!(
            digest,
            format!("content:sha256-v1:{}", sha256_digest_bytes(exact_bytes)),
            "{id}"
        );
        assert_eq!(byte_length, exact_bytes.len().to_string(), "{id}");
        assert_eq!(bytes, *exact_bytes, "{id}");
    }
}

struct CorpusDatabase {
    admin_pool: PgPool,
    pool: PgPool,
    schema: String,
}

impl CorpusDatabase {
    async fn create() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL is required for parity tests");
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect PostgreSQL test administrator");
        let schema = unique_schema();
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create isolated corpus schema");
        let options = database_url
            .parse::<PgConnectOptions>()
            .expect("parse DATABASE_URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("connect isolated corpus schema");
        MIGRATOR
            .run(&pool)
            .await
            .expect("migrate isolated corpus schema");
        Self {
            admin_pool,
            pool,
            schema,
        }
    }

    async fn validated_store_scope_id(&self) -> String {
        sqlx::query_scalar("SELECT store_scope_id FROM store_identity WHERE singleton")
            .fetch_one(&self.pool)
            .await
            .expect("load retained store scope")
    }

    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin_pool)
        .await
        .expect("drop isolated corpus schema");
        self.admin_pool.close().await;
    }
}

fn unique_schema() -> String {
    let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    format!(
        "mfm_corpus_{}_{}_{}",
        std::process::id(),
        timestamp,
        counter
    )
}
