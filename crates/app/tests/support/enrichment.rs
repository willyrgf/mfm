use super::*;

#[tokio::test]
async fn enrichment_publication_and_matching_admission_survive_configuration_deletion() {
    let provider = provider(1);
    let repository = Arc::new(LostPublicationAck::default());
    let backend = Arc::new(FaultStore::new());
    let composed = ComposedRuntime::compose(
        backend.clone(),
        BoundCapabilitySet::new(vec![(
            1,
            EvmEndpoint::new("alpha").unwrap(),
            provider.clone() as Arc<dyn EvmReadProvider>,
        )])
        .unwrap(),
    )
    .unwrap();
    let app = Application::from_parts(composed, repository.clone());
    let mut candidate = document_value(vec![(1, "alpha")], "portfolio-example");
    candidate["entry_point"] = serde_json::json!("mfm.portfolio/enrich@1");
    let candidate = ConfigDocument::new(serde_json::to_vec(&candidate).unwrap())
        .await
        .unwrap();
    let imported = app
        .import_config(config_name("candidates"), candidate)
        .await
        .unwrap();
    let enrichment_id = run_id(71);
    provider.blocked.store(true, Ordering::SeqCst);
    {
        let selected = selection(&imported);
        let starting = app.start_run(enrichment_id.clone(), &selected);
        tokio::pin!(starting);
        tokio::select! {
            _ = provider.entered.notified() => {}
            _ = &mut starting => panic!("blocked enrichment must remain in flight"),
        }
    }
    let pending_calls = provider.calls.load(Ordering::SeqCst);
    let pending = app
        .publish_enrichment(config_name("unresolved"), &enrichment_id)
        .await
        .expect_err("incomplete discovery cannot publish");
    assert_eq!(pending.code(), "invalid_enrichment");
    assert_eq!(provider.calls.load(Ordering::SeqCst), pending_calls);
    provider.blocked.store(false, Ordering::SeqCst);
    backend
        .indeterminate_next_append
        .store(true, Ordering::SeqCst);
    let resumed_start = app
        .start_run(enrichment_id.clone(), &selection(&imported))
        .await
        .err()
        .expect("ambiguous resumed start");
    let mfm_app::RunRequestError::AppendIndeterminate {
        recovery:
            mfm_app::RunRecovery::Start {
                config,
                run_id: retained_id,
            },
        last_observed,
    } = resumed_start
    else {
        panic!("start recovery envelope");
    };
    assert_eq!(config, *imported.config());
    assert_eq!(retained_id, enrichment_id);
    assert_eq!(last_observed.unwrap().head_sequence(), 3);

    let enriched =
        app.start_run(enrichment_id.clone(), &selection(&imported))
            .await
            .unwrap_or_else(|error| match error {
                mfm_app::RunRequestError::Invocation(
                    mfm_runtime::InvocationFailure::Execution { error, .. },
                ) => panic!("enrichment Runtime execution: {error:?}"),
                mfm_app::RunRequestError::Request(error) => {
                    panic!("enrichment Application request: {error:?}")
                }
                _ => panic!("enrichment invocation failed"),
            });
    assert!(matches!(enriched.run().state(), RunViewState::Succeeded(_)));
    app.delete_config(imported.config().name(), imported.config().digest())
        .await
        .unwrap();
    let calls = provider.calls.load(Ordering::SeqCst);
    repository.lose_next.store(true, Ordering::SeqCst);
    let lost = app
        .publish_enrichment(config_name("resolved"), &enrichment_id)
        .await
        .expect_err("lost publication acknowledgement");
    assert_eq!(lost.code(), "config_mutation_indeterminate");
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    let published = app
        .publish_enrichment(config_name("resolved"), &enrichment_id)
        .await
        .unwrap();
    assert!(matches!(published, ImportOutcome::Unchanged { .. }));
    let repeated = app
        .publish_enrichment(config_name("resolved"), &enrichment_id)
        .await
        .unwrap();
    assert!(matches!(repeated, ImportOutcome::Unchanged { .. }));
    assert_eq!(repeated.config(), published.config());
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    let retained = repository
        .inner
        .load_config(published.config().name(), published.config().digest())
        .await
        .unwrap()
        .unwrap();
    for mutation in ["head", "values", "route"] {
        let mut wire: serde_json::Value =
            serde_json::from_slice(retained.canonical_bytes()).unwrap();
        match mutation {
            "head" => {
                wire["input"]["provenance"]["head"] =
                    serde_json::json!(format!("content:sha256-v1:{}", "0".repeat(64)))
            }
            "values" => {
                wire["input"]["portfolio"]["collections"][0]["request"]["sources"][0]["address"] =
                    serde_json::json!("0x2222222222222222222222222222222222222222")
            }
            _ => wire["input"]["routes"][0]["endpoint_id"] = serde_json::json!("different"),
        }
        let forged = app
            .import_config(
                config_name(mutation),
                ConfigDocument::new(serde_json::to_vec(&wire).unwrap())
                    .await
                    .unwrap(),
            )
            .await
            .unwrap();
        let rejected = app
            .start_run(run_id(74), &selection(&forged))
            .await
            .err()
            .expect("untrusted linkage");
        assert_eq!(rejected.code(), "invalid_enrichment");
        assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
        assert!(matches!(
            app.read_run(&run_id(74)).await,
            Err(mfm_app::RunRequestError::Invocation(
                mfm_runtime::InvocationFailure::Execution {
                    error: mfm_runtime::RuntimeError::Absent,
                    ..
                }
            ))
        ));
    }
    let mut new_candidate = document_value(vec![(1, "alpha")], "portfolio-example");
    new_candidate["entry_point"] = serde_json::json!("mfm.portfolio/enrich@1");
    let new_candidate = app
        .import_config(
            config_name("candidates"),
            ConfigDocument::new(serde_json::to_vec(&new_candidate).unwrap())
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    let next_enrichment = app
        .start_run(run_id(73), &selection(&new_candidate))
        .await
        .unwrap();
    let next = app
        .publish_enrichment(config_name("resolved"), next_enrichment.run().run_id())
        .await
        .unwrap();
    assert!(matches!(next, ImportOutcome::Created { .. }));
    assert_ne!(next.config().digest(), published.config().digest());
    assert_eq!(
        repository
            .inner
            .load_config(published.config().name(), published.config().digest())
            .await
            .unwrap()
            .unwrap()
            .canonical_bytes(),
        retained.canonical_bytes()
    );
    assert_eq!(
        app.read_run(&enrichment_id).await.unwrap().head_digest(),
        enriched.run().head_digest()
    );
    app.delete_config(
        new_candidate.config().name(),
        new_candidate.config().digest(),
    )
    .await
    .unwrap();
    let dependent_id = run_id(72);
    let dependent = app
        .start_run(dependent_id.clone(), &selection(&published))
        .await
        .unwrap();
    assert!(matches!(
        dependent.run().state(),
        RunViewState::Succeeded(_)
    ));
    let wrong_schema = app
        .publish_enrichment(config_name("wrong-schema"), &dependent_id)
        .await
        .expect_err("snapshot is not enrichment");
    assert_eq!(wrong_schema.code(), "invalid_enrichment");
    let mut wrong_digest = published.config().digest().as_str().to_owned();
    let last = wrong_digest.pop().unwrap();
    wrong_digest.push(if last == '0' { '1' } else { '0' });
    let same_name_wrong_revision = ConfigSelection::new(
        published.config().name().clone(),
        ConfigDigest::parse(wrong_digest).unwrap(),
    );
    assert_eq!(
        app.start_run(dependent_id.clone(), &same_name_wrong_revision)
            .await
            .err()
            .expect("exact revision mismatch")
            .code(),
        "run_admission_conflict"
    );
    app.delete_config(published.config().name(), published.config().digest())
        .await
        .unwrap();
    let calls = provider.calls.load(Ordering::SeqCst);
    let recovered = app
        .start_run(dependent_id.clone(), &selection(&published))
        .await
        .unwrap();
    assert_eq!(recovered.run().head_digest(), dependent.run().head_digest());
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    let conflict = app
        .start_run(dependent_id, &selection(&imported))
        .await
        .err()
        .expect("conflicting selection");
    assert_eq!(conflict.code(), "run_admission_conflict");
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
}

#[derive(Default)]
struct LostPublicationAck {
    inner: MemoryConfigRepository,
    lose_next: std::sync::atomic::AtomicBool,
}
impl ConfigRepository for LostPublicationAck {
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult> {
        Box::pin(async move {
            let result = self.inner.import_config(revision).await?;
            if self.lose_next.swap(false, Ordering::SeqCst) {
                Err(ConfigRepositoryError::Indeterminate)
            } else {
                Ok(result)
            }
        })
    }
    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>> {
        self.inner.load_config(name, digest)
    }
    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>> {
        self.inner.list_configs()
    }
    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()> {
        self.inner.delete_config(name, digest)
    }
}
