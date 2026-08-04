//! Private catalog bridge for the small set of unavoidable dynamic SQL strings.

use crate::roles::TargetRoleKind;

use sqlx::AssertSqlSafe;

/// Reviewed schema-manifest query identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaManifestQuery {
    /// Relation and column manifest.
    Relation,
    /// Constraint manifest.
    Constraint,
    /// Index manifest.
    Index,
    /// Executable-object manifest.
    Executable,
    /// ACL manifest.
    Acl,
}

/// Reviewed transaction isolation identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionIsolation {
    /// Read-committed write transaction.
    ReadCommitted,
    /// Repeatable-read snapshot transaction.
    RepeatableRead,
}

/// Fixed structured-batch query shape owned by the PostgreSQL catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuredBatchQuery {
    /// Select every batch in one run prefix.
    Prefix,
    /// Select one append identity for ambiguity resolution.
    ByAppendRequest,
}

/// Builds a qualification-role statement from a closed role identity.
pub(crate) fn schema_set_role(role: &str) -> AssertSqlSafe<String> {
    role_statement(role, TargetRoleKind::Qualification)
}

/// Returns one reviewed schema-manifest query.
pub(crate) fn schema_role_grants(query: SchemaManifestQuery) -> AssertSqlSafe<&'static str> {
    let statement = match query {
        SchemaManifestQuery::Relation => crate::schema::RELATION_MANIFEST_SQL,
        SchemaManifestQuery::Constraint => crate::schema::CONSTRAINT_MANIFEST_SQL,
        SchemaManifestQuery::Index => crate::schema::INDEX_MANIFEST_SQL,
        SchemaManifestQuery::Executable => crate::schema::EXECUTABLE_MANIFEST_SQL,
        SchemaManifestQuery::Acl => crate::schema::ACL_MANIFEST_SQL,
    };
    AssertSqlSafe(statement)
}

/// Returns one reviewed transaction isolation statement.
pub(crate) fn transaction_isolation(
    isolation: TransactionIsolation,
) -> AssertSqlSafe<&'static str> {
    let statement = match isolation {
        TransactionIsolation::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        TransactionIsolation::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
    };
    AssertSqlSafe(statement)
}

/// Builds a transaction-local role statement from a closed managed role identity.
pub(crate) fn transaction_set_role(role: &str) -> AssertSqlSafe<String> {
    any_role_statement(role)
}

/// Builds a session qualification-role statement from a closed role identity.
pub(crate) fn session_set_role(role: &str) -> AssertSqlSafe<String> {
    role_statement(role, TargetRoleKind::Qualification)
}

/// Builds a managed-session probe statement from a closed role identity.
pub(crate) fn session_probe_role(role: &str) -> AssertSqlSafe<String> {
    if role.ends_with("_rrd") {
        role_statement(role, TargetRoleKind::RunReader)
    } else if role.ends_with("_rwr") {
        role_statement(role, TargetRoleKind::RunWriter)
    } else if role.ends_with("_crd") {
        role_statement(role, TargetRoleKind::ConfigurationReader)
    } else if role.ends_with("_cwr") {
        role_statement(role, TargetRoleKind::ConfigurationWriter)
    } else {
        AssertSqlSafe("SET LOCAL ROLE \"\"".to_owned())
    }
}

fn role_statement(role: &str, expected_kind: TargetRoleKind) -> AssertSqlSafe<String> {
    let valid = role.len() == 26
        && role.starts_with("mfm_t_")
        && role.ends_with(match expected_kind {
            TargetRoleKind::Owner => "_own",
            TargetRoleKind::Qualification => "_qlf",
            TargetRoleKind::RunReader => "_rrd",
            TargetRoleKind::RunWriter => "_rwr",
            TargetRoleKind::ConfigurationReader => "_crd",
            TargetRoleKind::ConfigurationWriter => "_cwr",
        })
        && role[6..22]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    let statement = if valid {
        format!("SET LOCAL ROLE \"{role}\"")
    } else {
        String::from("SET LOCAL ROLE \"\"")
    };
    AssertSqlSafe(statement)
}

fn any_role_statement(role: &str) -> AssertSqlSafe<String> {
    let valid = [
        TargetRoleKind::Owner,
        TargetRoleKind::Qualification,
        TargetRoleKind::RunReader,
        TargetRoleKind::RunWriter,
        TargetRoleKind::ConfigurationReader,
        TargetRoleKind::ConfigurationWriter,
    ]
    .into_iter()
    .any(|kind| {
        let expected = match kind {
            TargetRoleKind::Owner => "_own",
            TargetRoleKind::Qualification => "_qlf",
            TargetRoleKind::RunReader => "_rrd",
            TargetRoleKind::RunWriter => "_rwr",
            TargetRoleKind::ConfigurationReader => "_crd",
            TargetRoleKind::ConfigurationWriter => "_cwr",
        };
        role.len() == 26
            && role.starts_with("mfm_t_")
            && role.ends_with(expected)
            && role[6..22]
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    });
    let statement = if valid {
        format!("SET LOCAL ROLE \"{role}\"")
    } else {
        String::from("SET LOCAL ROLE \"\"")
    };
    AssertSqlSafe(statement)
}

/// The bounded batch-selection statement assembled by the store-owned loader.
pub(crate) fn structured_select_batches(
    query: StructuredBatchQuery,
) -> AssertSqlSafe<&'static str> {
    let statement = match query {
        StructuredBatchQuery::Prefix => {
            "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                    candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                    predecessor_commit_digest, head_commit_digest, batch_envelope_json \
               FROM run_history_batches WHERE run_id = $1 \
              ORDER BY run_history_batches.run_sequence"
        }
        StructuredBatchQuery::ByAppendRequest => {
            "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                    candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                    predecessor_commit_digest, head_commit_digest, batch_envelope_json \
               FROM run_history_batches \
              WHERE run_id = $1 AND append_request_id = $2"
        }
    };
    AssertSqlSafe(statement)
}

/// The fixed object-row insertion statement consumed by the bounded chunk writer.
pub(crate) fn structured_insert_object_rows() -> String {
    "INSERT INTO run_history_batch_objects ( \
                run_id, run_sequence, object_ordinal, object_type, content_schema_id, \
                content_digest, canonical_json \
             ) "
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{schema_set_role, session_probe_role, transaction_set_role};

    #[test]
    fn role_statements_accept_only_derived_target_roles() {
        assert_eq!(
            schema_set_role("mfm_t_0123456789abcdef_qlf").0,
            "SET LOCAL ROLE \"mfm_t_0123456789abcdef_qlf\""
        );
        assert_eq!(
            session_probe_role("mfm_t_0123456789abcdef_rrd").0,
            "SET LOCAL ROLE \"mfm_t_0123456789abcdef_rrd\""
        );
        assert_eq!(
            transaction_set_role("mfm_t_0123456789abcdef_cwr").0,
            "SET LOCAL ROLE \"mfm_t_0123456789abcdef_cwr\""
        );
    }

    #[test]
    fn role_statements_fail_closed_for_untrusted_text() {
        assert_eq!(
            schema_set_role("SET ROLE attacker").0,
            "SET LOCAL ROLE \"\""
        );
        assert_eq!(
            session_probe_role("mfm_t_0123456789abcdef_own").0,
            "SET LOCAL ROLE \"\""
        );
        assert_eq!(
            transaction_set_role("mfm_t_0123456789abcdef;DROP ROLE").0,
            "SET LOCAL ROLE \"\""
        );
    }
}
