use super::*;

#[tokio::test]
async fn start_rejects_empty_invocation_key_before_store_connection() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let request = tmp.path().join("request.json");
    std::fs::write(&request, "{}").expect("write request");

    let mut args = start_args(request);
    args.invocation_key = Some(String::new());

    let error = execute_internal(&args)
        .await
        .expect_err("empty invocation key rejects before store construction");
    assert_eq!(error.code, "InvocationKeyInvalid");
}

#[tokio::test]
async fn start_requires_database_before_entry_point_resolution() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let request = tmp.path().join("request.json");
    std::fs::write(&request, "{}").expect("write request");

    let error = execute_internal(&start_args(request))
        .await
        .expect_err("missing database url");
    assert_eq!(error.code, "MissingDatabaseUrl");
}

fn start_args(request: PathBuf) -> StartArgs {
    StartArgs {
        entry_point: "mfm.portfolio/portfolio_snapshot@1".to_owned(),
        request,
        invocation_key: None,
        runtime_config: None,
        stores: RunStoresArgs { database_url: None },
    }
}
