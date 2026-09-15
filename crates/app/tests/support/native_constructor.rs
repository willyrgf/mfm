use super::*;
use mfm_values::{MfmValue, Object};

type BalanceInput = mfm_evm::EvmBalanceContext<mfm_portfolio::PortfolioContinuation>;

#[tokio::test]
async fn shipping_metadata_constructor_cases_reach_native_materialization_after_schema_admission() {
    let backend = Arc::new(FaultStore::new());
    let provider = provider(1);
    *provider.mode.lock().unwrap() = ProviderMode::Blocked;
    let app = application_with_backend(&[(1, "alpha", provider.clone())], backend.clone());
    let imported = app
        .import_config(
            config_name("native-constructor"),
            document(vec![(1, "alpha")]).await,
        )
        .await
        .unwrap();
    let run = run_id(79);
    {
        let selected = selection(&imported);
        let starting = app.start_run(run.clone(), &selected);
        tokio::pin!(starting);
        tokio::select! {
            _ = provider.entered.notified() => {},
            result = &mut starting => panic!("expected blocked shipping Read: {}", result.is_ok()),
        }
    }
    let loaded = backend.load_run(&run, None).await.unwrap().unwrap();
    let frame = mfm_journal::decode_frame(loaded.latest()).unwrap();
    let commit: serde_json::Value = serde_json::from_slice(frame.payload().as_bytes()).unwrap();
    let original: Object =
        serde_json::from_value(commit["operation"]["succeeded"]["output"].clone()).unwrap();
    let mut input: serde_json::Value = serde_json::from_slice(original.canonical_bytes()).unwrap();
    let ordinary: BalanceInput = serde_json::from_slice(original.canonical_bytes()).unwrap();
    assert_eq!(
        serde_json::to_value(ordinary).unwrap(),
        serde_json::to_value(original.decode::<BalanceInput>().unwrap()).unwrap()
    );
    let calls_before = provider.calls.load(Ordering::SeqCst);
    for (correlation, mutation) in [
        (String::new(), 0),
        ("a".repeat(257), 0),
        ("changed".into(), 1),
        ("malformed".into(), 2),
        ("wrong-slot".into(), 4),
    ] {
        input["metadata"]["correlation"] = serde_json::json!(correlation);
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&input).unwrap())
                .unwrap();
        let object = Object::from_canonical(
            ContentRef::new(
                original.value_ref().schema_id().clone(),
                mfm_canonical::raw_content_digest(canonical.as_bytes()),
            )
            .unwrap(),
            canonical.as_bytes(),
        )
        .unwrap();
        object
            .admit(&BalanceInput::schema_descriptor().unwrap())
            .unwrap();
        let projected = if mutation != 0 {
            None
        } else {
            assert!(serde_json::from_slice::<BalanceInput>(object.canonical_bytes()).is_err());
            let native = object.decode::<BalanceInput>().err().unwrap();
            let projected = native.details().as_value().clone();
            assert_eq!(native.code(), "constructor_error");
            assert_eq!(projected["metadata"]["location"], "metadata.correlation");
            let expected = if correlation.is_empty() {
                serde_json::json!("empty_correlation")
            } else {
                serde_json::json!({"correlation_too_long":{"limit":256,"observed_bytes":257}})
            };
            assert_eq!(projected["metadata"]["source"], expected);
            Some(projected)
        };
        let mut current = commit.clone();
        let mut replacement = serde_json::to_value(&object).unwrap();
        if mutation != 0 {
            match mutation {
                1 => replacement["value_ref"] = serde_json::to_value(original.value_ref()).unwrap(),
                2 => replacement["value_ref"] = serde_json::json!("malformed-reference"),
                4 => {
                    replacement =
                        serde_json::to_value(Object::from_value(&mfm_program::NoParams).unwrap())
                            .unwrap()
                }
                _ => unreachable!(),
            }
        }
        if mutation != 0 {
            current["operation"]["succeeded"]["output"] = replacement;
        } else {
            replace_object(
                &mut current,
                &serde_json::to_value(&original).unwrap(),
                &replacement,
            );
        }
        let payload =
            PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&current).unwrap())
                .unwrap();
        let fixture = mfm_journal::seal_frame(
            &run,
            frame.run_sequence(),
            frame.previous_head_digest(),
            &payload,
        )
        .unwrap();
        let head = mfm_store::RunSummary::new(
            run.clone(),
            frame.run_sequence(),
            fixture.head_digest().clone(),
            loaded.head().total_bytes() - frame.canonical_bytes().len() as u64
                + fixture.canonical_bytes().len() as u64,
        )
        .unwrap();
        let fixture_store = Arc::new(ConstructorSnapshot(
            mfm_store::LoadedRun::new(
                head.clone(),
                loaded.admission().clone(),
                Arc::from(fixture.canonical_bytes()),
                None,
            )
            .unwrap(),
        ));
        let composed = ComposedRuntime::compose(
            fixture_store,
            BoundCapabilitySet::new(vec![(
                1,
                EvmEndpoint::new("alpha").unwrap(),
                provider.clone() as Arc<dyn EvmReadProvider>,
            )])
            .unwrap(),
        )
        .unwrap();
        let fixture_app =
            Application::from_parts(composed, Arc::new(MemoryConfigRepository::default()));
        if mutation != 0 {
            let failure = fixture_app.read_run(&run).await.err().unwrap();
            assert_eq!(failure.code(), "internal");
            let report = serde_json::to_value(
                mfm_app::SerializableClientError::for_run(&failure, &failure.to_string()).unwrap(),
            )
            .unwrap();
            let native = &report["invocation"]["cause"]["native"];
            assert_eq!(native["operation"], "restore");
            if mutation == 4 {
                assert_eq!(native["stage"], "execute");
                let identity = &native["cause"]["details"]["identity"];
                assert_ne!(identity["expected"], identity["actual"]);
            } else {
                assert_eq!(native["stage"], "decode");
                assert_eq!(native["cause"]["details"]["category"], "data");
                let reason = native["cause"]["details"]["message"].as_str().unwrap();
                assert!(!reason.is_empty());
                if mutation == 1 {
                    assert!(reason.contains("content_digest"));
                }
                assert!(native["cause"]["details"]["line"].as_u64().unwrap() > 0);
                assert!(native["cause"]["details"]["column"].as_u64().unwrap() > 0);
            }
            assert!(report["invocation"]["size_limit"].is_null());
            assert!(matches!(
                failure.request_error(),
                Some(mfm_app::RequestError::Internal)
            ));
            assert_eq!(provider.calls.load(Ordering::SeqCst), calls_before);
            continue;
        }
        assert_eq!(
            fixture_app.read_run(&run).await.unwrap().head_digest(),
            head.head_digest()
        );
        let failure = fixture_app.progress_run(&run).await.err().unwrap();
        assert_eq!(failure.code(), "internal");
        let message = failure.to_string();
        let report = serde_json::to_value(
            mfm_app::SerializableClientError::for_run(&failure, &message).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["invocation"]["cause"]["native"]["operation"],
            "read_prepare"
        );
        assert_eq!(report["invocation"]["cause"]["native"]["stage"], "decode");
        assert_eq!(
            report["invocation"]["cause"]["native"]["cause"]["details"],
            projected.unwrap()
        );
        assert_eq!(
            report["invocation"]["last_observed"]["head_digest"],
            serde_json::to_value(head.head_digest()).unwrap()
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), calls_before);
    }
}

struct ConstructorSnapshot(mfm_store::LoadedRun);
impl Store for ConstructorSnapshot {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<mfm_store::LoadedRun>, StoreError>> + Send + 'a>>
    {
        assert_eq!(run_id, self.0.head().run_id());
        assert!(probe.is_none());
        Box::pin(async {
            mfm_store::LoadedRun::new(
                self.0.head().clone(),
                self.0.admission().clone(),
                self.0.latest().clone(),
                None,
            )
            .map(Some)
        })
    }
    fn append_run<'a>(
        &'a self,
        _: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        panic!("native constructor failure must not append")
    }
}
impl mfm_store::RunIndex for ConstructorSnapshot {
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        _: RunPageLimit,
    ) -> Pin<
        Box<dyn Future<Output = Result<mfm_store::RunPage, mfm_store::RunIndexError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let items = if after.is_none_or(|after| after < self.0.head().run_id()) {
                vec![self.0.head().clone()]
            } else {
                vec![]
            };
            mfm_store::RunPage::new(items, None)
        })
    }
}

fn replace_object(
    value: &mut serde_json::Value,
    original: &serde_json::Value,
    replacement: &serde_json::Value,
) {
    if value == original {
        *value = replacement.clone();
        return;
    }
    match value {
        serde_json::Value::Object(fields) => {
            for value in fields.values_mut() {
                replace_object(value, original, replacement);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_object(value, original, replacement);
            }
        }
        _ => {}
    }
}
