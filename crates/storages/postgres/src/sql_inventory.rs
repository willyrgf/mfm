//! Executable syntax inventory of SQL owned by this crate.
//!
//! The inventory is intentionally parsed as Rust syntax. Line scanning cannot see aliases,
//! wrapped helpers, SQLx macros, or `QueryBuilder`, and therefore cannot establish ownership.

const CATALOG_FUNCTION_ALLOWLIST: &[&str] = &[
    "schema_set_role",
    "schema_role_grants",
    "transaction_isolation",
    "transaction_set_role",
    "session_set_role",
    "session_probe_role",
    "structured_select_batches",
    "structured_insert_object_rows",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use syn::parse::Parser;
    use syn::spanned::Spanned;
    use syn::visit::Visit;
    use syn::{Expr, ExprCall, ExprMacro, ExprMethodCall, ExprPath, ItemUse, Lit, Token, UseTree};

    use super::CATALOG_FUNCTION_ALLOWLIST;

    #[test]
    fn every_runtime_query_is_owned_by_the_allowlist() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut unowned = Vec::new();
        for entry in walkdir(&root) {
            let source = fs::read_to_string(&entry).expect("read source");
            let syntax = syn::parse_file(&source).expect("parse PostgreSQL source");
            let mut visitor = SqlConstructionVisitor {
                file: &entry,
                unowned: &mut unowned,
                call_stack: Vec::new(),
                query_aliases: BTreeSet::new(),
                builder_aliases: BTreeSet::new(),
                builder_bindings: BTreeSet::new(),
                query_glob_imported: false,
            };
            visitor.visit_file(&syntax);
        }
        assert!(
            unowned.is_empty(),
            "unowned dynamic SQL statements: {unowned:?}"
        );
    }

    #[test]
    fn inventory_covers_alias_macro_and_query_builder_forms() {
        let fixtures = [
            (
                "positive_alias",
                r#"fn f() { let _ = sqlx::query(crate::sql_catalog::schema_set_role("mfm_t_0123456789abcdef_qlf")); }"#,
                true,
            ),
            (
                "positive_builder",
                r#"fn f() { let mut b = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT 1 FROM run_history_batches"); let _ = b.build(); }"#,
                false,
            ),
            (
                "positive_fixed_literal",
                r#"fn f() { let _ = sqlx::query("SELECT 1"); }"#,
                true,
            ),
            (
                "negative_wrapper",
                r#"fn f() { let _ = helper(sqlx::query("SELECT 1")); }"#,
                false,
            ),
            (
                "negative_format",
                r#"fn f(x: &str) { let _ = sqlx::query(format!("SELECT {x}")); }"#,
                false,
            ),
            (
                "negative_concat",
                r#"fn f(x: &str) { let _ = sqlx::query(concat!("SELECT ", x)); }"#,
                false,
            ),
            (
                "negative_query_alias",
                r#"use sqlx::query as q; fn f() { let _ = q("SELECT 1"); }"#,
                false,
            ),
            (
                "negative_builder_alias",
                r#"use sqlx::QueryBuilder as Builder; fn f() { let _ = Builder::<sqlx::Postgres>::new("SELECT 1"); }"#,
                false,
            ),
            (
                "negative_value_alias",
                r#"fn f(sql: &str) { let q = sqlx::query; let _ = q(sql); }"#,
                false,
            ),
            (
                "negative_glob_import",
                r#"use sqlx::*; fn f(sql: &str) { let _ = query(sql); }"#,
                false,
            ),
            (
                "positive_checked_macro",
                r#"fn f() { let _ = sqlx::query!("SELECT 1"); }"#,
                true,
            ),
            (
                "negative_dynamic_macro",
                r#"fn f(x: &str) { let _ = sqlx::query!(concat!("SELECT ", x)); }"#,
                false,
            ),
            (
                "negative_wrapped_catalog",
                r#"fn f() { let _ = sqlx::query(helper(crate::sql_catalog::schema_set_role("mfm_t_0123456789abcdef_qlf"))); }"#,
                false,
            ),
        ];
        for (name, source, owned) in fixtures {
            let syntax = syn::parse_file(source).expect("fixture parses");
            let mut unowned = Vec::new();
            let file = PathBuf::from(name);
            let mut visitor = SqlConstructionVisitor {
                file: &file,
                unowned: &mut unowned,
                call_stack: Vec::new(),
                query_aliases: BTreeSet::new(),
                builder_aliases: BTreeSet::new(),
                builder_bindings: BTreeSet::new(),
                query_glob_imported: false,
            };
            visitor.visit_file(&syntax);
            assert_eq!(unowned.is_empty(), owned, "fixture {name}");
        }
    }

    struct SqlConstructionVisitor<'a> {
        file: &'a std::path::Path,
        unowned: &'a mut Vec<String>,
        call_stack: Vec<String>,
        query_aliases: BTreeSet<String>,
        builder_aliases: BTreeSet<String>,
        builder_bindings: BTreeSet<String>,
        query_glob_imported: bool,
    }

    impl<'ast> Visit<'ast> for SqlConstructionVisitor<'_> {
        fn visit_item_use(&mut self, item: &'ast ItemUse) {
            register_use_tree(
                &item.tree,
                &mut Vec::new(),
                &mut self.query_aliases,
                &mut self.builder_aliases,
            );
            if is_sqlx_glob(&item.tree) {
                self.query_glob_imported = true;
            }
            syn::visit::visit_item_use(self, item);
        }

        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                let segments = &path.path.segments;
                let terminal = segments.last().map(|segment| segment.ident.to_string());
                let is_query = terminal.as_deref().is_some_and(|name| {
                    matches!(name, "query" | "query_as" | "query_scalar")
                        || self.query_aliases.contains(name)
                }) && self.is_query_path(path);
                let is_builder = terminal.as_deref() == Some("new") && self.is_builder_path(path);
                let owned = if is_builder {
                    self.catalog_owned(call)
                } else {
                    self.static_query_owned(call) || self.catalog_owned(call)
                };
                if (is_query || is_builder) && !owned {
                    self.record(call.func.span());
                }
            }
            let terminal = if let syn::Expr::Path(path) = call.func.as_ref() {
                path.path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
            } else {
                None
            };
            if let Some(terminal) = terminal {
                self.call_stack.push(terminal);
            }
            syn::visit::visit_expr_call(self, call);
            if matches!(call.func.as_ref(), syn::Expr::Path(_)) {
                self.call_stack.pop();
            }
        }

        fn visit_expr_method_call(&mut self, method: &'ast ExprMethodCall) {
            let name = method.method.to_string();
            if matches!(name.as_str(), "push" | "push_unseparated") {
                let receiver_name = match method.receiver.as_ref() {
                    Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
                    _ => None,
                };
                let is_builder = receiver_name
                    .as_deref()
                    .is_some_and(|name| self.builder_bindings.contains(name) || name == "row");
                let fixed_fragment = name == "push_unseparated"
                    && method.args.len() == 1
                    && matches!(method.args.first(), Some(Expr::Lit(expr)) if matches!(&expr.lit, Lit::Str(value) if value.value() == "::numeric"));
                if is_builder && !fixed_fragment {
                    self.record(method.method.span());
                }
            }
            syn::visit::visit_expr_method_call(self, method);
        }

        fn visit_local(&mut self, local: &'ast syn::Local) {
            if let (syn::Pat::Ident(pattern), Some(init)) = (&local.pat, &local.init) {
                if let Expr::Call(call) = init.expr.as_ref() {
                    if let Expr::Path(path) = call.func.as_ref() {
                        if self.is_builder_path(path) {
                            self.builder_bindings.insert(pattern.ident.to_string());
                        }
                    }
                }
                if let Expr::Path(path) = init.expr.as_ref() {
                    let terminal = path
                        .path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string());
                    if path
                        .path
                        .segments
                        .first()
                        .is_some_and(|segment| segment.ident == "sqlx")
                        && terminal.as_deref().is_some_and(|name| {
                            matches!(name, "query" | "query_as" | "query_scalar")
                        })
                    {
                        self.query_aliases.insert(pattern.ident.to_string());
                    }
                }
            }
            syn::visit::visit_local(self, local);
        }

        fn visit_expr_macro(&mut self, expression: &'ast ExprMacro) {
            let path = &expression.mac.path;
            if path.segments.last().is_some_and(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "query" | "query_as" | "query_scalar"
                )
            }) && self.is_query_path_path(path)
                && !self.static_macro_owned(expression)
            {
                self.record(path.span());
            }
            syn::visit::visit_expr_macro(self, expression);
        }

        fn visit_expr_path(&mut self, path: &'ast ExprPath) {
            // Calls are classified by `visit_expr_call`; visiting paths here is
            // retained only for recursive traversal of nested expressions.
            syn::visit::visit_expr_path(self, path);
        }
    }

    impl SqlConstructionVisitor<'_> {
        fn is_query_path(&self, path: &ExprPath) -> bool {
            self.is_query_path_path(&path.path)
        }

        fn is_query_path_path(&self, path: &syn::Path) -> bool {
            path.segments
                .first()
                .is_some_and(|segment| segment.ident == "sqlx")
                || path.segments.last().is_some_and(|segment| {
                    self.query_aliases.contains(&segment.ident.to_string())
                        || (self.query_glob_imported
                            && matches!(
                                segment.ident.to_string().as_str(),
                                "query" | "query_as" | "query_scalar"
                            ))
                })
        }

        fn is_builder_path(&self, path: &ExprPath) -> bool {
            path.path.segments.iter().any(|segment| {
                segment.ident == "QueryBuilder"
                    || self.builder_aliases.contains(&segment.ident.to_string())
            })
        }

        fn catalog_owned(&self, call: &ExprCall) -> bool {
            let Some(Expr::Call(catalog_call)) = call.args.first() else {
                return false;
            };
            let Expr::Path(catalog_path) = catalog_call.func.as_ref() else {
                return false;
            };
            let segments: Vec<_> = catalog_path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            segments.len() == 3
                && segments[0] == "crate"
                && segments[1] == "sql_catalog"
                && CATALOG_FUNCTION_ALLOWLIST
                    .iter()
                    .any(|function| segments[2] == *function)
        }

        fn static_query_owned(&self, expression: &ExprCall) -> bool {
            // A direct literal is reviewed fixed SQL. Any computed SQL, including
            // a wrapper around a literal or a value alias, must go through the
            // private catalog path below.
            if !self.call_stack.iter().all(|parent| parent == "pin") {
                return false;
            }
            let Expr::Path(path) = expression.func.as_ref() else {
                return false;
            };
            if path
                .path
                .segments
                .first()
                .is_none_or(|segment| segment.ident != "sqlx")
            {
                return false;
            }
            matches!(expression.args.first(), Some(Expr::Lit(expr)) if matches!(&expr.lit, Lit::Str(_)))
        }

        fn static_macro_owned(&self, expression: &ExprMacro) -> bool {
            if expression
                .mac
                .path
                .segments
                .first()
                .is_none_or(|segment| segment.ident != "sqlx")
            {
                return false;
            }
            let args = syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated
                .parse2(expression.mac.tokens.clone());
            let Ok(args) = args else {
                return false;
            };
            let terminal = expression
                .mac
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string());
            let sql_index = if terminal.as_deref() == Some("query_as") {
                1
            } else {
                0
            };
            matches!(
                args.iter().nth(sql_index),
                Some(Expr::Lit(expr)) if matches!(&expr.lit, Lit::Str(_))
            )
        }

        fn record(&mut self, span: proc_macro2::Span) {
            self.unowned
                .push(format!("{}:{}", self.file.display(), span.start().line));
        }
    }

    fn register_use_tree(
        tree: &UseTree,
        prefix: &mut Vec<String>,
        query_aliases: &mut BTreeSet<String>,
        builder_aliases: &mut BTreeSet<String>,
    ) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                register_use_tree(&path.tree, prefix, query_aliases, builder_aliases);
                prefix.pop();
            }
            UseTree::Name(name) => {
                let mut full = prefix.clone();
                full.push(name.ident.to_string());
                register_import(
                    &full,
                    name.ident.to_string(),
                    query_aliases,
                    builder_aliases,
                );
            }
            UseTree::Rename(rename) => {
                let mut full = prefix.clone();
                full.push(rename.ident.to_string());
                register_import(
                    &full,
                    rename.rename.to_string(),
                    query_aliases,
                    builder_aliases,
                );
            }
            UseTree::Group(group) => {
                for tree in &group.items {
                    register_use_tree(tree, prefix, query_aliases, builder_aliases);
                }
            }
            UseTree::Glob(_) => {}
        }
    }

    fn register_import(
        full: &[String],
        alias: String,
        query_aliases: &mut BTreeSet<String>,
        builder_aliases: &mut BTreeSet<String>,
    ) {
        if full == ["sqlx", "query"]
            || full == ["sqlx", "query_as"]
            || full == ["sqlx", "query_scalar"]
        {
            query_aliases.insert(alias.clone());
        }
        if full == ["sqlx", "QueryBuilder"] {
            builder_aliases.insert(alias);
        }
    }

    fn is_sqlx_glob(tree: &UseTree) -> bool {
        match tree {
            UseTree::Path(path) if path.ident == "sqlx" => {
                matches!(path.tree.as_ref(), UseTree::Glob(_))
            }
            UseTree::Group(group) => group.items.iter().any(is_sqlx_glob),
            UseTree::Path(path) => is_sqlx_glob(&path.tree),
            _ => false,
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
