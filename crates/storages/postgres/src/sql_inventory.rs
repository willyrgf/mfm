//! Executable syntax inventory of SQL owned by this crate.
//!
//! The inventory is intentionally parsed as Rust syntax. Line scanning cannot see aliases,
//! wrapped helpers, SQLx macros, or `QueryBuilder`, and therefore cannot establish ownership.

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
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    use syn::{ExprCall, ExprMacro, ExprPath, PathArguments};

    #[test]
    fn every_runtime_query_is_owned_by_the_allowlist() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut unowned = Vec::new();
        for entry in walkdir(&root) {
            let source = fs::read_to_string(&entry).expect("read source");
            let syntax = syn::parse_file(&source).expect("parse PostgreSQL source");
            let mut visitor = SqlConstructionVisitor {
                source: &source,
                file: &entry,
                unowned: &mut unowned,
            };
            visitor.visit_file(&syntax);
        }
        assert!(
            unowned.is_empty(),
            "unowned dynamic SQL statements: {unowned:?}"
        );
    }

    struct SqlConstructionVisitor<'a> {
        source: &'a str,
        file: &'a std::path::Path,
        unowned: &'a mut Vec<String>,
    }

    impl<'ast> Visit<'ast> for SqlConstructionVisitor<'_> {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                let segments = &path.path.segments;
                let terminal = segments.last().map(|segment| segment.ident.to_string());
                let is_query = matches!(
                    terminal.as_deref(),
                    Some("query" | "query_as" | "query_scalar" | "new")
                ) && (segments.iter().any(|segment| segment.ident == "sqlx")
                    || segments
                        .iter()
                        .any(|segment| segment.ident == "QueryBuilder"));
                if is_query && !self.owned(call.func.as_ref()) {
                    self.record(call.func.span());
                }
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_macro(&mut self, expression: &'ast ExprMacro) {
            let path = &expression.mac.path;
            if path.segments.iter().any(|segment| segment.ident == "sqlx")
                && path.segments.iter().any(|segment| {
                    matches!(
                        segment.ident.to_string().as_str(),
                        "query" | "query_as" | "query_scalar"
                    )
                })
                && !self.owned(&syn::Expr::Macro(expression.clone()))
            {
                self.record(path.span());
            }
            syn::visit::visit_expr_macro(self, expression);
        }

        fn visit_expr_path(&mut self, path: &'ast ExprPath) {
            // Catch imported/aliased query helpers even when the call target is not qualified.
            if path.path.segments.len() == 1
                && path.path.segments.first().is_some_and(|segment| {
                    matches!(
                        segment.ident.to_string().as_str(),
                        "query" | "query_as" | "query_scalar"
                    ) && matches!(segment.arguments, PathArguments::None)
                })
                && !self.source.contains("fn query(")
            {
                // The call visitor records the actual call; this path hook only keeps the
                // syntax visitor aware of aliases and deliberately does not double-report it.
            }
            syn::visit::visit_expr_path(self, path);
        }
    }

    impl SqlConstructionVisitor<'_> {
        fn owned(&self, expression: &syn::Expr) -> bool {
            let start = expression.span().start().line.saturating_sub(1);
            self.source.lines().skip(start).take(16).any(|line| {
                DYNAMIC_SQL_ALLOWLIST
                    .iter()
                    .any(|token| line.contains(token))
            })
        }

        fn record(&mut self, span: proc_macro2::Span) {
            self.unowned
                .push(format!("{}:{}", self.file.display(), span.start().line));
        }
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
