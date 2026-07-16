use super::*;

#[tokio::test]
async fn start_rejects_empty_invocation_key_before_store_connection() {
    let mut args = start_args();
    args.invocation_key = Some(String::new());

    let error = execute_internal(&args)
        .await
        .expect_err("empty invocation key rejects before store construction");
    assert_eq!(error.code, "InvocationKeyInvalid");
}

#[tokio::test]
async fn start_requires_database_before_entry_point_resolution() {
    let error = execute_internal(&start_args())
        .await
        .expect_err("missing database url");
    assert_eq!(error.code, "MissingDatabaseUrl");
}

fn start_args() -> StartArgs {
    StartArgs {
        entry_point: "mfm.unknown/missing@1".to_owned(),
        target: "acme/primary".to_owned(),
        invocation_key: None,
        runtime_config: None,
        stores: RunStoresArgs { database_url: None },
    }
}
