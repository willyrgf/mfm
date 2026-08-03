//! Executable inventory of dynamic SQL owned by this crate.
//!
//! Fixed SQL migrates to checked `sqlx` macros over time. Every remaining runtime
//! `sqlx::query` shape must appear in [`DYNAMIC_SQL_ALLOWLIST`]. The verification
//! leaf fails when the source tree introduces an unowned raw query.

/// Reviewed dynamic SQL statement owners. Each entry is a unique substring that
/// identifies one owned query family in this crate's sources.
pub const DYNAMIC_SQL_ALLOWLIST: &[&str] = &[
    "SET LOCAL ROLE",
    "SET TRANSACTION ISOLATION LEVEL",
    "SET TRANSACTION READ",
    "pg_catalog.set_config",
    "pg_catalog.pg_advisory_xact_lock",
    "FROM target_authority",
    "FROM store_identity",
    "FROM store_schema_metadata",
    "FROM run_history_batches",
    "FROM run_history_heads",
    "FROM run_history_batch_objects",
    "FROM tenant_fact_heads",
    "FROM tenant_fact_publications",
    "FROM configuration_revisions",
    "FROM configuration_heads",
    "INSERT INTO run_history_batches",
    "INSERT INTO run_history_heads",
    "INSERT INTO run_history_batch_objects",
    "INSERT INTO tenant_fact_heads",
    "INSERT INTO tenant_fact_publications",
    "INSERT INTO configuration_revisions",
    "INSERT INTO configuration_heads",
    "UPDATE run_history_heads",
    "UPDATE tenant_fact_heads",
    "UPDATE configuration_heads",
    "FROM _sqlx_migrations",
    "FROM pg_catalog.pg_roles",
    "FROM pg_catalog.pg_auth_members",
    "FROM pg_catalog.pg_database",
    "pg_catalog.pg_has_role",
    "current_schema()",
    "current_database()",
    "AssertSqlSafe",
    "sqlx::query(statement)",
    "DYNAMIC_SQL_ALLOWLIST",
];

#[cfg(test)]
mod tests {
    use super::DYNAMIC_SQL_ALLOWLIST;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn every_runtime_query_is_owned_by_the_allowlist() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut unowned = Vec::new();
        for entry in walkdir(&root) {
            let source = fs::read_to_string(&entry).expect("read source");
            for (line_no, line) in source.lines().enumerate() {
                if line.contains("sqlx::query(") || line.contains("sqlx::query_scalar(") {
                    let owned = DYNAMIC_SQL_ALLOWLIST.iter().any(|token| {
                        // Inspect a short window after the query call for ownership tokens.
                        source
                            .lines()
                            .skip(line_no)
                            .take(12)
                            .any(|window| window.contains(token))
                    });
                    if !owned {
                        unowned.push(format!("{}:{}", entry.display(), line_no + 1));
                    }
                }
            }
        }
        assert!(
            unowned.is_empty(),
            "unowned dynamic SQL statements: {unowned:?}"
        );
    }

    fn walkdir(root: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in fs::read_dir(root).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                files.extend(walkdir(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
        files
    }
}
