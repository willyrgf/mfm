use std::collections::BTreeSet;

use mfm_canonical::{CanonicalValue, RecoverabilityContractV3};
use mfm_journal::v2::*;
use mfm_values::component_object_evidence_contract_ref;
use serde_json::Value;

const CORPUS_BYTES: &[u8] = include_bytes!("../../../../contracts/recoverability/v3/corpus.json");

fn corpus() -> Value {
    serde_json::from_slice(CORPUS_BYTES).expect("frozen corpus must decode")
}

fn hex_bytes(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(pair, 16).expect("valid corpus hex")
        })
        .collect()
}

fn schema_goldens(contract: &str) -> Vec<Vec<u8>> {
    corpus()["positive_vectors"]
        .as_array()
        .expect("positive vector array")
        .iter()
        .filter(|vector| {
            vector["kind"].as_str() == Some("schema_acceptance")
                && vector["schema_contract"].as_str() == Some(contract)
        })
        .map(|vector| hex_bytes(vector["canonical_hex"].as_str().expect("canonical hex")))
        .collect()
}

fn schema_golden(contract: &str) -> Vec<u8> {
    let goldens = schema_goldens(contract);
    assert_eq!(goldens.len(), 1, "{contract} must have one golden here");
    goldens.into_iter().next().expect("one golden")
}

fn assert_exact_rebuild(
    original: &impl PersistedJournalValue,
    rebuilt: &impl PersistedJournalValue,
) {
    assert_eq!(rebuilt.schema_id(), original.schema_id());
    assert_eq!(rebuilt.as_bytes(), original.as_bytes());
}

fn assert_unknown_field_rejected<T, E>(
    contract: &str,
    decode: fn(&[u8]) -> std::result::Result<T, E>,
) where
    T: std::fmt::Debug,
    E: std::fmt::Debug + std::fmt::Display,
{
    let mut value: Value =
        serde_json::from_slice(&schema_goldens(contract)[0]).expect("schema golden JSON");
    value
        .as_object_mut()
        .expect("tested schema is an object")
        .insert(
            "provider_message".to_owned(),
            Value::String("provider-secret-sentinel".to_owned()),
        );
    let bytes = serde_json::to_vec(&value).expect("mutated JSON");
    let error = decode(&bytes).expect_err("unknown fields must reject");
    assert!(
        !error.to_string().contains("provider-secret-sentinel"),
        "rejection must not echo provider-controlled text"
    );
}

fn assert_exact_retained_contract_rebuild(
    original: &RetainedValueContract,
    rebuilt: &RetainedValueContract,
) {
    assert_eq!(rebuilt.schema_id(), original.schema_id());
    assert_eq!(
        rebuilt
            .canonical_json()
            .expect("rebuilt retained contract canonical JSON"),
        original
            .canonical_json()
            .expect("original retained contract canonical JSON")
    );
}

#[test]
fn persisted_non_domain_failure_constructors_preserve_capability_values() {
    let failure = NonDomainFailure::new(
        NonDomainEntryStatus::MayHaveEntered,
        NonDomainDisposition::RetryableOperational,
        NonDomainFailureCode::ExecutorContention,
    )
    .expect("closed non-domain failure");
    let persisted = PersistedNonDomainFailure::new(&failure).expect("persisted wrapper");
    assert_eq!(persisted.value().expect("projected failure"), failure);
    assert_eq!(
        PersistedNonDomainEntryStatus::new(failure.entry_status())
            .expect("persisted entry status")
            .value()
            .expect("projected entry status"),
        failure.entry_status()
    );
    assert_eq!(
        PersistedNonDomainDisposition::new(failure.disposition())
            .expect("persisted disposition")
            .value()
            .expect("projected disposition"),
        failure.disposition()
    );
    assert_eq!(
        PersistedNonDomainFailureCode::new(failure.code())
            .expect("persisted code")
            .value()
            .expect("projected code"),
        failure.code()
    );
}

#[test]
fn every_added_codec_strictly_rejects_open_fields() {
    assert_unknown_field_rejected("mfm.safe-failure.v1", SafeFailure::strict_decode);
    assert_unknown_field_rejected("mfm.input-source.v1", InputSource::strict_decode);
    assert_unknown_field_rejected("mfm.input-binding.v1", InputBinding::strict_decode);
    assert_unknown_field_rejected("mfm.input-manifest.v1", InputManifest::strict_decode);
    assert_unknown_field_rejected(
        "mfm.root-manifest-entry.v1",
        RootManifestEntry::strict_decode,
    );
    assert_unknown_field_rejected("mfm.config-manifest.v1", ConfigManifest::strict_decode);
    assert_unknown_field_rejected("mfm.seed-manifest.v1", SeedManifest::strict_decode);
    assert_unknown_field_rejected("mfm.context-manifest.v1", ContextManifest::strict_decode);
    assert_unknown_field_rejected(
        "mfm.cross-run-source-manifest-entry.v1",
        CrossRunSourceManifestEntry::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.cross-run-source-manifest.v1",
        CrossRunSourceManifest::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.retained-value-contract.v1",
        RetainedValueContract::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.configured-value-key.v1",
        ConfiguredValueKey::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.configured-value-binding.v1",
        ConfiguredValueBinding::strict_decode,
    );
    assert_unknown_field_rejected("mfm.value-ref.v1", ValueRef::strict_decode);
    assert_unknown_field_rejected("mfm.producer-binding.v1", ProducerBinding::strict_decode);
    assert_unknown_field_rejected(
        "mfm.object-path-binding.v1",
        ObjectPathBinding::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.artifact-admission-intent.v1",
        ArtifactAdmissionIntent::strict_decode,
    );
    assert_unknown_field_rejected("mfm.source-object-ref.v1", SourceObjectRef::strict_decode);
    assert_unknown_field_rejected(
        "mfm.fact-claim-envelope.v1",
        FactClaimEnvelope::strict_decode,
    );
    assert_unknown_field_rejected("mfm.fact-emission.v1", FactEmission::strict_decode);
    assert_unknown_field_rejected(
        "mfm.external-access-authorized.v1",
        ExternalAccessAuthorized::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.external-access-observed.v2",
        ExternalAccessObserved::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.fact-selection-scan-attestation.v1",
        FactSelectionScanAttestation::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.fact-selection-scan-contract.v1",
        FactSelectionScanContract::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.source-closure-preimage.v1",
        SourceClosurePreimage::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.binding-delta-entry.v1",
        BindingDeltaEntry::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.blocking-destination.v1",
        BlockingDestination::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.blocking-producer-requirement.v1",
        BlockingProducerRequirement::strict_decode,
    );
    assert_unknown_field_rejected("mfm.blocking-source.v1", BlockingSource::strict_decode);
    assert_unknown_field_rejected(
        "mfm.node-terminal-outcome.v1",
        NodeTerminalOutcome::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.node-semantic-state.v1",
        NodeSemanticState::strict_decode,
    );
    assert_unknown_field_rejected("mfm.semantic-binding.v1", SemanticBinding::strict_decode);
    assert_unknown_field_rejected(
        "mfm.pending-effect-state.v1",
        PendingEffectState::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.run-semantic-state-preimage.v1",
        RunSemanticStatePreimage::strict_decode,
    );
    assert_unknown_field_rejected("mfm.frozen-read-intent.v1", FrozenReadIntent::strict_decode);
    assert_unknown_field_rejected(
        "mfm.read-capability-binding.v1",
        ReadCapabilityBinding::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.executor-proof-basis.v1",
        ExecutorProofBasis::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.terminal-effect-evidence.v1",
        TerminalEffectEvidence::strict_decode,
    );
    assert_unknown_field_rejected(
        "mfm.executor-ensure-result.v1",
        ExecutorEnsureResult::strict_decode,
    );
}

#[test]
fn safe_failure_constructor_and_projection_match_the_frozen_annex() {
    let original =
        SafeFailure::strict_decode(&schema_golden("mfm.safe-failure.v1")).expect("safe failure");
    let fields = original.fields().expect("typed safe-failure fields");
    let rebuilt = SafeFailure::new(
        &fields.safe_failure_contract_ref,
        &fields.stable_code,
        fields.failure_class,
        fields.boundary_stage,
        fields.coarse_size_class,
        fields.diagnostic_ref.as_ref(),
    )
    .expect("typed safe-failure constructor");

    assert_exact_rebuild(&original, &rebuilt);

    let keys = serde_json::from_slice::<Value>(rebuilt.as_bytes())
        .expect("canonical JSON")
        .as_object()
        .expect("safe failure object")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        BTreeSet::from([
            "boundary_stage".to_owned(),
            "coarse_size_class".to_owned(),
            "diagnostic_ref".to_owned(),
            "failure_class".to_owned(),
            "safe_failure_contract_ref".to_owned(),
            "stable_code".to_owned(),
            "version".to_owned(),
        ])
    );
}

#[test]
fn safe_failure_strict_decode_rejects_every_prohibited_open_diagnostic_field() {
    let original = schema_golden("mfm.safe-failure.v1");
    let sentinel = "provider-secret-sentinel";

    for prohibited in [
        "provider_message",
        "provider_body",
        "url",
        "credential",
        "path",
        "debug",
        "source_chain",
    ] {
        let mut value: Value = serde_json::from_slice(&original).expect("safe-failure JSON");
        value
            .as_object_mut()
            .expect("safe-failure object")
            .insert(prohibited.to_owned(), Value::String(sentinel.to_owned()));
        let bytes = serde_json::to_vec(&value).expect("mutated JSON");
        let error = match SafeFailure::strict_decode(&bytes) {
            Ok(_) => panic!("{prohibited} must not be retained"),
            Err(error) => error,
        };
        assert!(
            !error.to_string().contains(sentinel),
            "rejection must not echo provider-controlled text"
        );
    }
}

#[test]
fn admission_manifest_constructors_match_the_frozen_objects() {
    let root = RootManifestEntry::strict_decode(&schema_golden("mfm.root-manifest-entry.v1"))
        .expect("root manifest entry");
    let rebuilt_root = RootManifestEntry::new(
        root.field_path().expect("root field path"),
        root.value_ref().expect("root value ref"),
    )
    .expect("root manifest entry constructor");
    assert_exact_rebuild(&root, &rebuilt_root);

    let config = ConfigManifest::strict_decode(&schema_golden("mfm.config-manifest.v1"))
        .expect("config manifest");
    let rebuilt_config =
        ConfigManifest::new(config.entries().expect("config entries")).expect("config constructor");
    assert_exact_rebuild(&config, &rebuilt_config);

    let seed =
        SeedManifest::strict_decode(&schema_golden("mfm.seed-manifest.v1")).expect("seed manifest");
    let rebuilt_seed =
        SeedManifest::new(seed.entries().expect("seed entries")).expect("seed constructor");
    assert_exact_rebuild(&seed, &rebuilt_seed);

    let context = ContextManifest::strict_decode(&schema_golden("mfm.context-manifest.v1"))
        .expect("context manifest");
    let rebuilt_context = ContextManifest::new(context.entries().expect("context entries"))
        .expect("context constructor");
    assert_exact_rebuild(&context, &rebuilt_context);

    let source_entry = CrossRunSourceManifestEntry::strict_decode(&schema_golden(
        "mfm.cross-run-source-manifest-entry.v1",
    ))
    .expect("cross-run source manifest entry");
    let rebuilt_source_entry = CrossRunSourceManifestEntry::new(
        source_entry.field_path().expect("source field path"),
        source_entry.source().expect("cross-run source"),
    )
    .expect("cross-run source entry constructor");
    assert_exact_rebuild(&source_entry, &rebuilt_source_entry);

    let sources =
        CrossRunSourceManifest::strict_decode(&schema_golden("mfm.cross-run-source-manifest.v1"))
            .expect("cross-run source manifest");
    let rebuilt_sources =
        CrossRunSourceManifest::new(sources.entries().expect("cross-run source entries"))
            .expect("cross-run source manifest constructor");
    assert_exact_rebuild(&sources, &rebuilt_sources);
}

#[test]
fn journal_runtime_retained_contract_factories_are_exact_and_deterministic() {
    struct ManifestContractCase {
        schema_contract: &'static str,
        semantic_type_id: &'static str,
        role: &'static str,
        factory: fn() -> mfm_journal::v2::Result<RetainedValueContract>,
    }

    let recoverability =
        RecoverabilityContractV3::embedded().expect("embedded recoverability contract");
    let evidence_ref =
        component_object_evidence_contract_ref().expect("component object evidence ref");
    let cases = [
        ManifestContractCase {
            schema_contract: "mfm.input-manifest.v1",
            semantic_type_id: "semantic:mfm.recoverability:input-manifest:1:sha256-jcs-v1:e1df430d25f440aca6d6edf69f84c7be13c8a1755995f3e6e155c364401664e3",
            role: "mfm.admission.input-manifest",
            factory: InputManifest::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.config-manifest.v1",
            semantic_type_id: "semantic:mfm.recoverability:config-manifest:1:sha256-jcs-v1:d5d56121fd22135d33b88943f6cea22d38b99ad4b5e384181926439c4bdfed00",
            role: "mfm.admission.config-manifest",
            factory: ConfigManifest::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.seed-manifest.v1",
            semantic_type_id: "semantic:mfm.recoverability:seed-manifest:1:sha256-jcs-v1:71de769ef39bcafdb1306bf42cff2469efde990d68017d686215bf4c5681cbd1",
            role: "mfm.admission.seed-manifest",
            factory: SeedManifest::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.context-manifest.v1",
            semantic_type_id: "semantic:mfm.recoverability:context-manifest:1:sha256-jcs-v1:552facb07cba48bd7a7e9da719f02ad03fec00c1deb4131b1238f48051352575",
            role: "mfm.admission.context-manifest",
            factory: ContextManifest::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.cross-run-source-manifest.v1",
            semantic_type_id: "semantic:mfm.recoverability:cross-run-source-manifest:1:sha256-jcs-v1:ea2e18af0e31551bfbbf6aac6d88d3419d9ff01c669550a74e4631cb9330eb05",
            role: "mfm.admission.cross-run-source-manifest",
            factory: CrossRunSourceManifest::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.fact-claim-envelope.v1",
            semantic_type_id: "semantic:mfm.recoverability:fact-claim-envelope:1:sha256-jcs-v1:69e0c1833c98402873bf1eb87003255b964d8aa5cdec2750694d4a8b8e52f7df",
            role: "mfm.runtime.fact-claim-envelope",
            factory: FactClaimEnvelope::retained_contract,
        },
        ManifestContractCase {
            schema_contract: "mfm.frozen-read-intent.v1",
            semantic_type_id: "semantic:mfm.recoverability:frozen-read-intent:1:sha256-jcs-v1:4ee1cd3e6f280a54428c2a092d259a25c7b798362f0154b3f778a969ca79b8ba",
            role: "mfm.runtime.frozen-read-intent",
            factory: FrozenReadIntent::retained_contract,
        },
    ];

    for case in cases {
        let retained = (case.factory)().expect("manifest retained contract");
        assert_eq!(
            retained.schema_id(),
            recoverability
                .schema_id(case.schema_contract)
                .expect("manifest schema")
        );
        assert_eq!(retained.semantic_type_id().as_str(), case.semantic_type_id);
        assert_eq!(retained.role().as_str(), case.role);
        assert_eq!(retained.media_type(), "application/json");
        assert_eq!(retained.evidence_contract_ref(), &evidence_ref);
        assert_eq!(
            (case.factory)().expect("repeated manifest retained contract"),
            retained
        );
    }
}

#[test]
fn admission_manifest_constructors_sort_and_bound_unique_paths() {
    let root = RootManifestEntry::strict_decode(&schema_golden("mfm.root-manifest-entry.v1"))
        .expect("root manifest entry");
    let value_ref = root.value_ref().expect("root value ref");
    let first = RootManifestEntry::new(
        mfm_ids::FieldPath::new("a").expect("first path"),
        value_ref.clone(),
    )
    .expect("first root");
    let second = RootManifestEntry::new(
        mfm_ids::FieldPath::new("b").expect("second path"),
        value_ref,
    )
    .expect("second root");

    let sorted = ConfigManifest::new(vec![second.clone(), first.clone()])
        .expect("constructor sorts roots")
        .entries()
        .expect("sorted roots");
    assert_eq!(
        sorted
            .iter()
            .map(|entry| entry.field_path().expect("field path"))
            .collect::<Vec<_>>(),
        vec![
            mfm_ids::FieldPath::new("a").expect("first path"),
            mfm_ids::FieldPath::new("b").expect("second path"),
        ]
    );
    assert!(
        ConfigManifest::new(vec![first.clone(), first.clone()]).is_err(),
        "duplicate root paths must reject"
    );
    assert!(
        ConfigManifest::new(vec![first; 4097]).is_err(),
        "root manifests are bounded to 4096 entries"
    );

    let source_entry = CrossRunSourceManifestEntry::strict_decode(&schema_golden(
        "mfm.cross-run-source-manifest-entry.v1",
    ))
    .expect("cross-run source manifest entry");
    let source = source_entry.source().expect("cross-run source");
    let first = CrossRunSourceManifestEntry::new(
        mfm_ids::FieldPath::new("a").expect("first source path"),
        source.clone(),
    )
    .expect("first source");
    let second = CrossRunSourceManifestEntry::new(
        mfm_ids::FieldPath::new("b").expect("second source path"),
        source,
    )
    .expect("second source");
    let sorted = CrossRunSourceManifest::new(vec![second, first.clone()])
        .expect("constructor sorts sources")
        .entries()
        .expect("sorted sources");
    assert_eq!(
        sorted
            .iter()
            .map(|entry| entry.field_path().expect("source field path"))
            .collect::<Vec<_>>(),
        vec![
            mfm_ids::FieldPath::new("a").expect("first source path"),
            mfm_ids::FieldPath::new("b").expect("second source path"),
        ]
    );
    assert!(
        CrossRunSourceManifest::new(vec![first.clone(), first]).is_err(),
        "duplicate source paths must reject"
    );
}

#[test]
fn every_input_source_constructor_matches_its_frozen_union_variant() {
    let mut kinds = BTreeSet::new();

    for bytes in schema_goldens("mfm.input-source.v1") {
        let original = InputSource::strict_decode(&bytes).expect("input source");
        let (kind, rebuilt) = match original.fields().expect("typed input source") {
            InputSourceFields::RunAdmission { record_ref } => {
                ("run_admission", InputSource::run_admission(&record_ref))
            }
            InputSourceFields::TransitionOutput {
                this_run_id,
                transition_ref,
                output_ref,
            } => (
                "transition_output",
                InputSource::transition_output(&this_run_id, &transition_ref, &output_ref),
            ),
            InputSourceFields::TransitionFact {
                this_run_id,
                transition_ref,
                fact_ref,
            } => (
                "transition_fact",
                InputSource::transition_fact(&this_run_id, &transition_ref, &fact_ref),
            ),
            InputSourceFields::CrossRun { source_ref } => {
                ("cross_run", InputSource::cross_run(&source_ref))
            }
            InputSourceFields::Config { value_ref } => ("config", InputSource::config(&value_ref)),
            InputSourceFields::QualifiedSupport {
                member_path,
                value_ref,
            } => (
                "qualified_support",
                InputSource::qualified_support(&member_path, &value_ref),
            ),
            InputSourceFields::Seed { value_ref } => ("seed", InputSource::seed(&value_ref)),
            InputSourceFields::Context { value_ref } => {
                ("context", InputSource::context(&value_ref))
            }
        };
        let rebuilt = rebuilt.expect("typed input-source constructor");
        assert_exact_rebuild(&original, &rebuilt);
        kinds.insert(kind);
    }

    assert_eq!(
        kinds,
        BTreeSet::from([
            "config",
            "context",
            "cross_run",
            "qualified_support",
            "run_admission",
            "seed",
            "transition_fact",
            "transition_output",
        ])
    );
}

#[test]
fn every_cross_run_source_variant_has_complete_typed_lineage() {
    let mut kinds = BTreeSet::new();

    for bytes in schema_goldens("mfm.cross-run-source-ref.v1") {
        let source = CrossRunSourceRef::strict_decode(&bytes).expect("cross-run source");
        let common = source.identity().expect("common source identity");
        match source.fields().expect("typed source role") {
            CrossRunSourceRefFields::EffectiveOutput {
                identity,
                effective_transition_ref,
                effective_output_ref,
            } => {
                assert_eq!(identity, common);
                assert_eq!(
                    effective_output_ref
                        .fields()
                        .expect("effective output coordinate")
                        .transition_ref,
                    effective_transition_ref
                );
                kinds.insert("effective_output");
            }
            CrossRunSourceRefFields::EvidenceOnly {
                identity,
                raw_transition_ref,
                raw_result_or_evidence_ref,
                certified_evidence_role_ref,
            } => {
                assert_eq!(identity, common);
                raw_transition_ref
                    .fields()
                    .expect("raw transition coordinate");
                raw_result_or_evidence_ref
                    .fields()
                    .expect("raw result or evidence authority");
                assert_eq!(
                    certified_evidence_role_ref.content_digest().algorithm(),
                    mfm_ids::DigestAlgorithm::Sha256V1
                );
                kinds.insert("evidence_only");
            }
        }
    }

    assert_eq!(kinds, BTreeSet::from(["effective_output", "evidence_only"]));
}

#[test]
fn input_binding_and_manifest_constructors_match_the_frozen_objects() {
    let original_binding =
        InputBinding::strict_decode(&schema_golden("mfm.input-binding.v1")).expect("input binding");
    let binding = original_binding.fields().expect("typed input binding");
    let rebuilt_binding = InputBinding::new(
        &binding.field_path,
        &binding.source,
        binding.source_field_path.as_ref(),
        &binding.value_ref,
    )
    .expect("typed input-binding constructor");
    assert_exact_rebuild(&original_binding, &rebuilt_binding);

    let fact_source = schema_goldens("mfm.input-source.v1")
        .into_iter()
        .map(|bytes| InputSource::strict_decode(&bytes).expect("input source"))
        .find(|source| {
            matches!(
                source.fields().expect("typed input source"),
                InputSourceFields::TransitionFact { .. }
            )
        })
        .expect("transition-fact source");
    assert!(
        InputBinding::new(
            &binding.field_path,
            &fact_source,
            Some(&binding.field_path),
            &binding.value_ref,
        )
        .is_err(),
        "a complete fact root cannot carry source field selection"
    );

    let original_manifest = InputManifest::strict_decode(&schema_golden("mfm.input-manifest.v1"))
        .expect("input manifest");
    let manifest = original_manifest.fields().expect("typed input manifest");
    let rebuilt_manifest = InputManifest::new(
        &manifest.input_schema_id,
        &manifest.root_input_ref,
        manifest.config_ref.as_ref(),
        manifest.context_ref.as_ref(),
        &manifest.bindings,
    )
    .expect("typed input-manifest constructor");
    assert_exact_rebuild(&original_manifest, &rebuilt_manifest);

    assert!(
        InputManifest::new(
            &manifest.input_schema_id,
            &manifest.root_input_ref,
            manifest.config_ref.as_ref(),
            manifest.context_ref.as_ref(),
            &[rebuilt_binding.clone(), rebuilt_binding],
        )
        .is_err(),
        "duplicate field-path bindings violate the frozen unique ordering"
    );
}

#[test]
fn every_producer_binding_constructor_matches_its_frozen_union_variant() {
    let mut kinds = BTreeSet::new();

    for bytes in schema_goldens("mfm.producer-binding.v1") {
        let original = ProducerBinding::strict_decode(&bytes).expect("producer binding");
        let (kind, rebuilt) = match original.fields().expect("typed producer binding") {
            ProducerBindingFields::RunAdmission {
                record_ref,
                field_path,
            } => (
                "run_admission",
                ProducerBinding::run_admission(&record_ref, &field_path),
            ),
            ProducerBindingFields::ThisAdmission { slot } => {
                ("this_admission", ProducerBinding::this_admission(&slot))
            }
            ProducerBindingFields::ThisRecord { field_path } => {
                ("this_record", ProducerBinding::this_record(&field_path))
            }
            ProducerBindingFields::InputAssembly { run_id, node_id } => (
                "input_assembly",
                ProducerBinding::input_assembly(&run_id, &node_id),
            ),
            ProducerBindingFields::TransitionOutput {
                run_id,
                node_id,
                output_ordinal,
            } => (
                "transition_output",
                ProducerBinding::transition_output(&run_id, &node_id, output_ordinal),
            ),
            ProducerBindingFields::TransitionFact {
                run_id,
                node_id,
                emission_ordinal,
                component,
            } => (
                "transition_fact",
                ProducerBinding::transition_fact(&run_id, &node_id, emission_ordinal, component),
            ),
            ProducerBindingFields::PublicOutputAssembly { run_id } => (
                "public_output_assembly",
                ProducerBinding::public_output_assembly(&run_id),
            ),
            ProducerBindingFields::QualifiedSupport {
                qualification_scope_id,
                field_path,
            } => (
                "qualified_support",
                ProducerBinding::qualified_support(&qualification_scope_id, &field_path),
            ),
            ProducerBindingFields::ConfiguredValue {
                store_scope_id,
                tenant_scope_id,
                entry_point_id,
                target,
            } => (
                "configured_value",
                ProducerBinding::configured_value(
                    &store_scope_id,
                    &tenant_scope_id,
                    &entry_point_id,
                    &target,
                ),
            ),
            ProducerBindingFields::ExternalObservation {
                authorization_ref,
                field_path,
            } => (
                "external_observation",
                ProducerBinding::external_observation(&authorization_ref, &field_path),
            ),
            ProducerBindingFields::SourceRun { source_ref } => {
                ("source_run", ProducerBinding::source_run(&source_ref))
            }
        };
        let rebuilt = rebuilt.expect("typed producer-binding constructor");
        assert_exact_rebuild(&original, &rebuilt);
        kinds.insert(kind);
    }

    assert_eq!(
        kinds,
        BTreeSet::from([
            "external_observation",
            "configured_value",
            "input_assembly",
            "public_output_assembly",
            "qualified_support",
            "run_admission",
            "source_run",
            "this_admission",
            "this_record",
            "transition_fact",
            "transition_output",
        ])
    );
}

#[test]
fn retained_object_authority_constructors_preserve_every_evidence_field() {
    let original_contract =
        RetainedValueContract::strict_decode(&schema_golden("mfm.retained-value-contract.v1"))
            .expect("retained-value contract");
    let rebuilt_contract = RetainedValueContract::new(
        original_contract.schema_id().clone(),
        original_contract.semantic_type_id().clone(),
        original_contract.role().clone(),
        original_contract.media_type(),
        original_contract.evidence_contract_ref().clone(),
    )
    .expect("typed retained-value contract constructor");
    assert_exact_retained_contract_rebuild(&original_contract, &rebuilt_contract);

    let original_value =
        ValueRef::strict_decode(&schema_golden("mfm.value-ref.v1")).expect("value ref");
    let value = original_value.fields().expect("typed value ref");
    let rebuilt_value = ValueRef::new(
        &value.artifact_id,
        &value.content_digest,
        &value.evidence_hash,
        &value.schema_id,
        &value.semantic_type_id,
        &value.role,
        value.byte_length,
        &value.media_type,
        &value.evidence_contract_ref,
        &value.producer_binding,
    )
    .expect("typed value-ref constructor");
    assert_exact_rebuild(&original_value, &rebuilt_value);

    let exact_contract = RetainedValueContract::new(
        value.schema_id.clone(),
        value.semantic_type_id.clone(),
        value.role.clone(),
        &value.media_type,
        value.evidence_contract_ref.clone(),
    )
    .expect("contract from exact value metadata");
    original_value
        .validate_contract(&exact_contract)
        .expect("contract matches value metadata");

    let original_binding =
        ObjectPathBinding::strict_decode(&schema_golden("mfm.object-path-binding.v1"))
            .expect("object path binding");
    let binding = original_binding
        .fields()
        .expect("typed object path binding");
    let rebuilt_binding = ObjectPathBinding::new(
        binding.record_ordinal,
        &binding.field_path,
        binding.authority_use,
        &binding.value_ref,
        &binding.evidence_contract_ref,
    )
    .expect("typed object-path-binding constructor");
    assert_exact_rebuild(&original_binding, &rebuilt_binding);

    let original_intent =
        ArtifactAdmissionIntent::strict_decode(&schema_golden("mfm.artifact-admission-intent.v1"))
            .expect("artifact admission intent");
    let intent = original_intent
        .fields()
        .expect("typed artifact admission intent");
    let rebuilt_intent = ArtifactAdmissionIntent::new(
        &intent.value_ref,
        &intent.evidence_contract_ref,
        intent.mode,
    )
    .expect("typed artifact-admission constructor");
    assert_exact_rebuild(&original_intent, &rebuilt_intent);

    let wrong_contract = original_value
        .content_ref()
        .expect("different valid content ref");
    assert!(
        ObjectPathBinding::new(
            binding.record_ordinal,
            &binding.field_path,
            binding.authority_use,
            &binding.value_ref,
            &wrong_contract,
        )
        .is_err(),
        "a path binding cannot contradict its nested evidence contract"
    );
    assert!(
        ArtifactAdmissionIntent::new(&intent.value_ref, &wrong_contract, intent.mode).is_err(),
        "an admission intent cannot contradict its nested evidence contract"
    );

    let source = SourceObjectRef::strict_decode(&schema_golden("mfm.source-object-ref.v1"))
        .expect("source object ref");
    let source = source.fields().expect("typed source object ref");
    assert!(
        !source.evidence_contract_ref.schema_id().as_str().is_empty(),
        "source authority retains its evidence contract"
    );
}

#[test]
fn blocking_lineage_constructors_match_every_closed_variant() {
    let mut destinations = BTreeSet::new();
    for bytes in schema_goldens("mfm.blocking-destination.v1") {
        let original =
            BlockingDestination::strict_decode(&bytes).expect("blocking destination golden");
        let (kind, rebuilt) = match original.fields().expect("typed blocking destination") {
            BlockingDestinationFields::Config => ("config", BlockingDestination::config()),
            BlockingDestinationFields::Context => ("context", BlockingDestination::context()),
            BlockingDestinationFields::Input {
                destination_field_path,
            } => ("input", BlockingDestination::input(&destination_field_path)),
        };
        assert_exact_rebuild(
            &original,
            &rebuilt.expect("blocking destination constructor"),
        );
        destinations.insert(kind);
    }
    assert_eq!(destinations, BTreeSet::from(["config", "context", "input"]));

    let mut requirements = BTreeSet::new();
    for bytes in schema_goldens("mfm.blocking-producer-requirement.v1") {
        let original = BlockingProducerRequirement::strict_decode(&bytes)
            .expect("blocking requirement golden");
        let (kind, rebuilt) = match original.fields().expect("typed blocking requirement") {
            BlockingProducerRequirementFields::Output {
                output_ordinal,
                source_field_path,
            } => (
                "output",
                BlockingProducerRequirement::output(output_ordinal, source_field_path.as_ref()),
            ),
            BlockingProducerRequirementFields::Fact { emission_ordinal } => {
                ("fact", BlockingProducerRequirement::fact(emission_ordinal))
            }
        };
        assert_exact_rebuild(
            &original,
            &rebuilt.expect("blocking requirement constructor"),
        );
        requirements.insert(kind);
    }
    assert_eq!(requirements, BTreeSet::from(["fact", "output"]));

    let original = BlockingSource::strict_decode(&schema_golden("mfm.blocking-source.v1"))
        .expect("blocking source golden");
    let fields = original.fields().expect("typed blocking source");
    let rebuilt = BlockingSource::new(
        &fields.producer_node_id,
        &fields.producer_terminal_transition_ref,
        &fields.destination,
        &fields.requirement,
    )
    .expect("blocking source constructor");
    assert_exact_rebuild(&original, &rebuilt);
}

#[test]
fn fact_claim_envelope_constructor_matches_the_frozen_object() {
    let original = FactClaimEnvelope::strict_decode(&schema_golden("mfm.fact-claim-envelope.v1"))
        .expect("fact claim envelope");
    let fields = original.fields().expect("typed fact claim envelope");
    let rebuilt = FactClaimEnvelope::new(
        &fields.fact_descriptor_ref,
        &fields.subject_ref,
        &fields.response_ref,
    )
    .expect("typed fact claim constructor");
    assert_exact_rebuild(&original, &rebuilt);
}

#[test]
fn fact_emission_constructor_preserves_actual_and_certified_slot_ordinals() {
    let original =
        FactEmission::strict_decode(&schema_golden("mfm.fact-emission.v1")).expect("fact emission");
    let fields = original.fields().expect("typed fact emission");
    let rebuilt = FactEmission::new(
        fields.emission_ordinal,
        fields.fact_slot_ordinal,
        &fields.fact_descriptor_ref,
        &fields.claim_ref,
        &fields.fact_content_identity,
    )
    .expect("typed fact-emission constructor");

    assert_exact_rebuild(&original, &rebuilt);
}

#[test]
fn successful_settlement_requires_dense_emissions_grouped_by_fact_slot() {
    let original =
        FactEmission::strict_decode(&schema_golden("mfm.fact-emission.v1")).expect("fact emission");
    let fields = original.fields().expect("typed fact emission");
    let emission = |emission_ordinal, fact_slot_ordinal| {
        FactEmission::new(
            emission_ordinal,
            fact_slot_ordinal,
            &fields.fact_descriptor_ref,
            &fields.claim_ref,
            &fields.fact_content_identity,
        )
        .expect("fact emission")
    };

    Settlement::succeeded(
        &[],
        &[
            emission(0, 3),
            emission(1, 3),
            emission(2, 8),
            emission(3, 8),
        ],
    )
    .expect("dense emissions grouped by nondecreasing fact slot");

    assert!(
        Settlement::succeeded(&[], &[emission(0, 3), emission(2, 3)]).is_err(),
        "actual emission ordinals must be dense"
    );
    assert!(
        Settlement::succeeded(&[], &[emission(0, 3), emission(1, 8), emission(2, 3)]).is_err(),
        "emissions cannot return to an earlier fact slot"
    );

    let structurally_valid = Settlement::from_canonical_value(
        CanonicalValue::object([
            ("kind", CanonicalValue::String("succeeded".to_owned())),
            ("output_bindings", CanonicalValue::Array(Vec::new())),
            (
                "fact_emissions",
                CanonicalValue::Array(
                    [emission(0, 3), emission(2, 3)]
                        .iter()
                        .map(PersistedJournalValue::canonical_value)
                        .collect::<Result<Vec<_>>>()
                        .expect("canonical fact emissions"),
                ),
            ),
        ])
        .expect("settlement object"),
    )
    .expect("structurally valid settlement");
    assert!(
        structurally_valid.fields().is_err(),
        "the closed projector must reject a persisted non-dense sequence"
    );
}

#[test]
fn every_binding_delta_entry_constructor_matches_its_frozen_union_variant() {
    let mut kinds = BTreeSet::new();

    for bytes in schema_goldens("mfm.binding-delta-entry.v1") {
        let original = BindingDeltaEntry::strict_decode(&bytes).expect("binding delta entry");
        let (kind, rebuilt) = match original.fields().expect("typed binding delta") {
            BindingDeltaEntryFields::NodePhaseChange { node_id, phase } => (
                "node_phase_change",
                BindingDeltaEntry::node_phase_change(&node_id, phase),
            ),
            BindingDeltaEntryFields::OutputBinding(binding) => (
                "output_binding",
                BindingDeltaEntry::output_binding(&binding),
            ),
            BindingDeltaEntryFields::FactBinding(emission) => {
                ("fact_binding", BindingDeltaEntry::fact_binding(&emission))
            }
            BindingDeltaEntryFields::PendingEffectInsert {
                effect_key,
                request_digest,
            } => (
                "pending_effect_insert",
                BindingDeltaEntry::pending_effect_insert(&effect_key, &request_digest),
            ),
            BindingDeltaEntryFields::PendingEffectRemove { effect_key } => (
                "pending_effect_remove",
                BindingDeltaEntry::pending_effect_remove(&effect_key),
            ),
            BindingDeltaEntryFields::PublicOutputChange(output_ref) => (
                "public_output_change",
                BindingDeltaEntry::public_output_change(output_ref.as_ref()),
            ),
            BindingDeltaEntryFields::RunPhaseChange(run_phase) => (
                "run_phase_change",
                BindingDeltaEntry::run_phase_change(run_phase),
            ),
        };
        let rebuilt = rebuilt.expect("typed binding-delta constructor");
        assert_exact_rebuild(&original, &rebuilt);
        kinds.insert(kind);
    }

    assert_eq!(
        kinds,
        BTreeSet::from([
            "fact_binding",
            "node_phase_change",
            "output_binding",
            "pending_effect_insert",
            "pending_effect_remove",
            "public_output_change",
            "run_phase_change",
        ])
    );
}

#[test]
fn typed_semantic_state_components_match_every_frozen_variant() {
    let mut outcomes = BTreeSet::new();
    for bytes in schema_goldens("mfm.node-terminal-outcome.v1") {
        let original = NodeTerminalOutcome::strict_decode(&bytes).expect("node terminal outcome");
        let (kind, rebuilt) = match original.fields().expect("typed node terminal outcome") {
            NodeTerminalOutcomeFields::Succeeded => ("succeeded", NodeTerminalOutcome::succeeded()),
            NodeTerminalOutcomeFields::Failed { typed_failure_ref } => {
                ("failed", NodeTerminalOutcome::failed(&typed_failure_ref))
            }
            NodeTerminalOutcomeFields::Skipped { blocking_sources } => {
                ("skipped", NodeTerminalOutcome::skipped(&blocking_sources))
            }
        };
        let rebuilt = rebuilt.expect("typed node-terminal-outcome constructor");
        assert_exact_rebuild(&original, &rebuilt);
        outcomes.insert(kind);
    }
    assert_eq!(outcomes, BTreeSet::from(["failed", "skipped", "succeeded"]));

    let original_node =
        NodeSemanticState::strict_decode(&schema_golden("mfm.node-semantic-state.v1"))
            .expect("node semantic state");
    let node = original_node.fields().expect("typed node semantic state");
    let rebuilt_node =
        NodeSemanticState::new(&node.node_id, node.phase, node.terminal_outcome.as_ref())
            .expect("typed node-semantic-state constructor");
    assert_exact_rebuild(&original_node, &rebuilt_node);

    let mut bindings = BTreeSet::new();
    for bytes in schema_goldens("mfm.semantic-binding.v1") {
        let original = SemanticBinding::strict_decode(&bytes).expect("semantic binding");
        let (kind, rebuilt) = match original.fields().expect("typed semantic binding") {
            SemanticBindingFields::Input {
                node_id,
                field_path,
                value_ref,
            } => (
                "input",
                SemanticBinding::input(&node_id, &field_path, &value_ref),
            ),
            SemanticBindingFields::Output {
                node_id,
                output_ordinal,
                value_ref,
            } => (
                "output",
                SemanticBinding::output(&node_id, output_ordinal, &value_ref),
            ),
            SemanticBindingFields::Fact {
                node_id,
                emission_ordinal,
                claim_ref,
                subject_ref,
                response_ref,
            } => (
                "fact",
                SemanticBinding::fact(
                    &node_id,
                    emission_ordinal,
                    &claim_ref,
                    &subject_ref,
                    &response_ref,
                ),
            ),
        };
        let rebuilt = rebuilt.expect("typed semantic-binding constructor");
        assert_exact_rebuild(&original, &rebuilt);
        bindings.insert(kind);
    }
    assert_eq!(bindings, BTreeSet::from(["fact", "input", "output"]));

    let original_pending =
        PendingEffectState::strict_decode(&schema_golden("mfm.pending-effect-state.v1"))
            .expect("pending effect state");
    let pending = original_pending
        .fields()
        .expect("typed pending effect state");
    let rebuilt_pending = PendingEffectState::new(
        &pending.node_id,
        &pending.effect_key,
        &pending.request_digest,
        &pending.executor_binding_ref,
        &pending.input_manifest_ref,
        &pending.semantic_request_ref,
    )
    .expect("typed pending-effect-state constructor");
    assert_exact_rebuild(&original_pending, &rebuilt_pending);

    let original_terminal =
        RunTerminalContract::strict_decode(&schema_golden("mfm.run-terminal-contract.v1"))
            .expect("run terminal contract");
    let terminal = original_terminal
        .fields()
        .expect("typed run terminal contract");
    let rebuilt_terminal = RunTerminalContract::new(
        &terminal.required_success_nodes,
        terminal.requires_public_output,
    )
    .expect("typed run-terminal-contract constructor");
    assert_exact_rebuild(&original_terminal, &rebuilt_terminal);

    let original_manifest_ref =
        InputManifestRef::strict_decode(&schema_golden("mfm.input-manifest-ref.v1"))
            .expect("input manifest ref");
    let manifest = original_manifest_ref
        .fields()
        .expect("typed input manifest ref");
    let rebuilt_manifest_ref =
        InputManifestRef::new(&manifest.value_ref).expect("typed input-manifest-ref constructor");
    assert_exact_rebuild(&original_manifest_ref, &rebuilt_manifest_ref);
}

#[test]
fn semantic_state_preimage_constructor_is_an_exact_annex_codec() {
    let original = RunSemanticStatePreimage::strict_decode(&schema_golden(
        "mfm.run-semantic-state-preimage.v1",
    ))
    .expect("run semantic-state preimage");
    let fields = original.fields().expect("typed outer preimage fields");
    let rebuilt = RunSemanticStatePreimage::new(
        &fields.spec_hash,
        fields.run_phase,
        &fields.ordered_node_states,
        &fields.typed_bindings,
        &fields.pending_effects,
        fields.public_output.as_ref(),
    )
    .expect("semantic-state preimage codec");
    assert_exact_rebuild(&original, &rebuilt);

    assert!(
        RunSemanticStatePreimage::new(
            &fields.spec_hash,
            fields.run_phase,
            &[],
            &fields.typed_bindings,
            &fields.pending_effects,
            fields.public_output.as_ref(),
        )
        .is_err(),
        "the frozen annex requires at least one ordered node state"
    );

    let duplicate = fields.ordered_node_states[0].clone();
    assert!(
        RunSemanticStatePreimage::new(
            &fields.spec_hash,
            fields.run_phase,
            &[duplicate.clone(), duplicate],
            &fields.typed_bindings,
            &fields.pending_effects,
            fields.public_output.as_ref(),
        )
        .is_err(),
        "the frozen annex rejects duplicate node-state components"
    );
}

#[test]
fn frozen_read_intent_constructor_matches_the_frozen_object() {
    let original = FrozenReadIntent::strict_decode(&schema_golden("mfm.frozen-read-intent.v1"))
        .expect("frozen read intent");
    let fields = original.fields().expect("typed frozen read intent");
    let rebuilt = FrozenReadIntent::new(
        &fields.node_id,
        &fields.input_manifest_ref,
        &fields.state_contract_ref,
        &fields.capability_binding_ref,
        &fields.routing_generation_ref,
        &fields.capability_operation_id,
        &fields.request_ref,
        &fields.request_digest,
        &fields.request_contract,
        &fields.returned_contract,
        &fields.safe_failure_contract,
    )
    .expect("typed frozen-read-intent constructor");
    assert_exact_rebuild(&original, &rebuilt);
}

#[test]
fn external_authorization_constructor_freezes_read_intent_exactly_for_reads() {
    let mut saw_read = false;
    let mut fixture = None;

    for bytes in schema_goldens("mfm.external-access-authorized.v1") {
        let original =
            ExternalAccessAuthorized::strict_decode(&bytes).expect("external authorization");
        let fields = original.fields().expect("typed external authorization");
        match fields.scope.fields().expect("typed authorization scope") {
            AuthorizationScopeFields::Read { .. } => {
                saw_read = true;
                fixture.get_or_insert(fields.clone());
            }
            AuthorizationScopeFields::EnsureEffect { .. } => {}
        }
        let rebuilt = ExternalAccessAuthorized::new(
            &fields.semantic_anchor,
            &fields.scope,
            &fields.capability_binding_ref,
            &fields.capability_operation_id,
            &fields.request_ref,
            fields.frozen_read_intent_ref.as_ref(),
        )
        .expect("typed external-authorization constructor");
        assert_exact_rebuild(&original, &rebuilt);

        if matches!(
            fields.scope.fields().expect("typed authorization scope"),
            AuthorizationScopeFields::Read { .. }
        ) {
            assert!(
                ExternalAccessAuthorized::new(
                    &fields.semantic_anchor,
                    &fields.scope,
                    &fields.capability_binding_ref,
                    &fields.capability_operation_id,
                    &fields.request_ref,
                    None,
                )
                .is_err(),
                "read authorization cannot omit its complete frozen intent"
            );
        }
    }

    assert!(saw_read, "corpus covers read authorization");

    let fields = fixture.expect("authorization fixture");
    let transition_record =
        RecordRef::strict_decode(&schema_golden("mfm.record-ref.v1")).expect("transition record");
    let transition_ref = TransitionRef::new(&transition_record).expect("transition ref");
    let ensure_scope =
        AuthorizationScope::ensure_effect(&transition_ref).expect("ensure authorization scope");
    let ensure = ExternalAccessAuthorized::new(
        &fields.semantic_anchor,
        &ensure_scope,
        &fields.capability_binding_ref,
        &fields.capability_operation_id,
        &fields.request_ref,
        None,
    )
    .expect("typed effect-ensure authorization");
    assert!(matches!(
        ensure
            .fields()
            .expect("typed ensure authorization")
            .scope
            .fields()
            .expect("typed ensure scope"),
        AuthorizationScopeFields::EnsureEffect { .. }
    ));
    assert!(
        ExternalAccessAuthorized::new(
            &fields.semantic_anchor,
            &ensure_scope,
            &fields.capability_binding_ref,
            &fields.capability_operation_id,
            &fields.request_ref,
            fields.frozen_read_intent_ref.as_ref(),
        )
        .is_err(),
        "effect authorization cannot carry a frozen read intent"
    );
}

#[test]
fn fact_scan_attestation_and_observation_constructors_preserve_authority_links() {
    let original_attestation = FactSelectionScanAttestation::strict_decode(&schema_golden(
        "mfm.fact-selection-scan-attestation.v1",
    ))
    .expect("fact scan attestation");
    let attestation = original_attestation
        .fields()
        .expect("typed fact scan attestation");
    let rebuilt_attestation = FactSelectionScanAttestation::new(
        &attestation.store_scope_id,
        &attestation.store_epoch,
        &attestation.tenant_scope_id,
        &attestation.authorization_ref,
        &attestation.request_digest,
        &attestation.frontier,
        &attestation.response_ref,
        &attestation.response_closure_digest,
    )
    .expect("typed fact scan attestation constructor");
    assert_exact_rebuild(&original_attestation, &rebuilt_attestation);

    let mut saw_attested_return = false;
    for bytes in schema_goldens("mfm.external-access-observed.v2") {
        let original = ExternalAccessObserved::strict_decode(&bytes).expect("external observation");
        let fields = original.fields().expect("typed external observation");
        saw_attested_return |= fields.fact_selection_scan_attestation_ref.is_some();
        let rebuilt = ExternalAccessObserved::new(
            &fields.authorization_ref,
            &fields.outcome,
            fields.fact_selection_scan_attestation_ref.as_ref(),
        )
        .expect("typed external-observation constructor");
        assert_exact_rebuild(&original, &rebuilt);
    }
    assert!(
        saw_attested_return,
        "corpus covers an authoritative fact-selection observation"
    );
}

#[test]
fn fact_scan_and_read_capability_contract_constructors_preserve_frozen_metadata() {
    let original_scan = FactSelectionScanContract::strict_decode(&schema_golden(
        "mfm.fact-selection-scan-contract.v1",
    ))
    .expect("fact scan contract");
    let scan = original_scan.fields().expect("typed fact scan contract");
    let rebuilt_scan = FactSelectionScanContract::new(
        &scan.capability_operation_id,
        &scan.request_contract,
        &scan.response_contract,
        &scan.scan_attestation_contract,
    )
    .expect("typed fact scan contract constructor");
    assert_exact_rebuild(&original_scan, &rebuilt_scan);

    let original_binding =
        ReadCapabilityBinding::strict_decode(&schema_golden("mfm.read-capability-binding.v1"))
            .expect("read capability binding");
    let binding = original_binding
        .fields()
        .expect("typed read capability binding");
    let rebuilt_binding = ReadCapabilityBinding::new(
        &binding.capability_contract_ref,
        &binding.admitted_implementation_ref,
        &binding.safe_classifier_contract_ref,
        &binding.safe_failure_contract_ref,
        &binding.reviewed_source_scope_ref,
        &binding.routing_catalog_ref,
    )
    .expect("typed read capability binding constructor");
    assert_exact_rebuild(&original_binding, &rebuilt_binding);

    let mut legacy: Value =
        serde_json::from_slice(original_binding.as_bytes()).expect("binding canonical JSON");
    let legacy = legacy.as_object_mut().expect("binding object");
    let routing = legacy
        .remove("routing_catalog_ref")
        .expect("frozen routing catalog");
    legacy.insert("routing_generation_ref".to_owned(), routing);
    assert!(
        ReadCapabilityBinding::strict_decode(
            &serde_json::to_vec(&legacy).expect("legacy binding JSON"),
        )
        .is_err(),
        "the superseded per-call routing generation field has no alias"
    );
}

#[test]
fn configured_value_and_source_closure_constructors_preserve_complete_authority() {
    let original_key =
        ConfiguredValueKey::strict_decode(&schema_golden("mfm.configured-value-key.v1"))
            .expect("configured value key");
    let key = original_key.fields().expect("typed configured value key");
    let rebuilt_key = ConfiguredValueKey::new(
        &key.store_scope_id,
        &key.tenant_scope_id,
        &key.entry_point_id,
        &key.target,
    )
    .expect("typed configured value key constructor");
    assert_exact_rebuild(&original_key, &rebuilt_key);

    let original_binding =
        ConfiguredValueBinding::strict_decode(&schema_golden("mfm.configured-value-binding.v1"))
            .expect("configured value binding");
    let binding = original_binding
        .fields()
        .expect("typed configured value binding");
    let rebuilt_binding = ConfiguredValueBinding::new(&binding.key, &binding.value_ref)
        .expect("typed configured value binding constructor");
    assert_exact_rebuild(&original_binding, &rebuilt_binding);

    let original_closure =
        SourceClosurePreimage::strict_decode(&schema_golden("mfm.source-closure-preimage.v1"))
            .expect("source closure preimage");
    let closure = original_closure
        .fields()
        .expect("typed source closure preimage");
    let rebuilt_closure = SourceClosurePreimage::new(
        &closure.root_source_manifest_ref,
        &closure.ordered_dependency_refs,
        &closure.ordered_object_refs,
    )
    .expect("typed source closure preimage constructor");
    assert_exact_rebuild(&original_closure, &rebuilt_closure);
    let rebuilt_digest: mfm_ids::SourceClosureDigest = rebuilt_closure
        .source_closure_digest()
        .expect("source closure digest");
    let original_digest: mfm_ids::SourceClosureDigest = original_closure
        .source_closure_digest()
        .expect("source closure digest");
    assert_eq!(
        rebuilt_digest, original_digest,
        "the exact golden preimage must derive one branded closure identity"
    );
}

#[test]
fn every_executor_proof_basis_constructor_matches_its_frozen_union_variant() {
    let self_authenticating =
        ExecutorProofBasis::self_authenticating_proof().expect("self-authenticating proof");
    let self_authenticating_round_trip =
        ExecutorProofBasis::strict_decode(self_authenticating.as_bytes())
            .expect("self-authenticating proof round trip");
    assert_exact_rebuild(&self_authenticating, &self_authenticating_round_trip);
    assert!(matches!(
        self_authenticating_round_trip
            .fields()
            .expect("typed self-authenticating proof"),
        ExecutorProofBasisFields::SelfAuthenticatingProof
    ));

    let mut kinds = BTreeSet::from(["self_authenticating_proof"]);

    for bytes in schema_goldens("mfm.executor-proof-basis.v1") {
        let original = ExecutorProofBasis::strict_decode(&bytes).expect("executor proof basis");
        let (kind, rebuilt) = match original.fields().expect("typed executor proof basis") {
            ExecutorProofBasisFields::SelfAuthenticatingProof => (
                "self_authenticating_proof",
                ExecutorProofBasis::self_authenticating_proof(),
            ),
            ExecutorProofBasisFields::ExecutorAttestation {
                evidence_authority_ref,
            } => (
                "executor_attestation",
                ExecutorProofBasis::executor_attestation(&evidence_authority_ref),
            ),
            ExecutorProofBasisFields::TrustedObserver {
                evidence_authority_ref,
            } => (
                "trusted_observer",
                ExecutorProofBasis::trusted_observer(&evidence_authority_ref),
            ),
        };
        let rebuilt = rebuilt.expect("typed executor-proof-basis constructor");
        assert_exact_rebuild(&original, &rebuilt);
        kinds.insert(kind);
    }

    assert_eq!(
        kinds,
        BTreeSet::from([
            "executor_attestation",
            "self_authenticating_proof",
            "trusted_observer",
        ])
    );
}

#[test]
fn terminal_effect_evidence_constructor_matches_the_frozen_object() {
    let original =
        TerminalEffectEvidence::strict_decode(&schema_golden("mfm.terminal-effect-evidence.v1"))
            .expect("terminal effect evidence");
    let fields = original.fields().expect("typed terminal effect evidence");
    let rebuilt = TerminalEffectEvidence::new(
        &fields.executor_binding_ref,
        &fields.effect_key,
        &fields.request_digest,
        &fields.delivery_audit_ref,
        &fields.terminal_tombstone_ref,
        &fields.external_operation_identity,
        &fields.terminal_outcome,
        &fields.assurance_policy_ref,
        &fields.proof_basis,
        &fields.domain_evidence_ref,
    )
    .expect("typed terminal-effect-evidence constructor");
    assert_exact_rebuild(&original, &rebuilt);
}

#[test]
fn every_executor_ensure_result_constructor_matches_its_frozen_union_variant() {
    let mut kinds = BTreeSet::new();

    for bytes in schema_goldens("mfm.executor-ensure-result.v1") {
        let original = ExecutorEnsureResult::strict_decode(&bytes).expect("executor ensure result");
        let (kind, rebuilt) = match original.fields().expect("typed ensure result") {
            ExecutorEnsureResultFields::Pending { delivery_audit_ref } => (
                "pending",
                ExecutorEnsureResult::pending(&delivery_audit_ref),
            ),
            ExecutorEnsureResultFields::Terminal { evidence_ref } => {
                ("terminal", ExecutorEnsureResult::terminal(&evidence_ref))
            }
        };
        let rebuilt = rebuilt.expect("typed executor-ensure-result constructor");
        assert_exact_rebuild(&original, &rebuilt);
        kinds.insert(kind);
    }

    assert_eq!(kinds, BTreeSet::from(["pending", "terminal"]));
}

#[test]
fn access_audit_delivery_head_is_present_only_for_returned_ensure() {
    let audit = AccessAuditEntry::strict_decode(&schema_golden("mfm.access-audit-entry.v2"))
        .expect("access audit");
    let authorization_ref = audit
        .fields()
        .expect("access audit fields")
        .authorization_ref;
    let observation_record =
        RecordRef::strict_decode(&schema_golden("mfm.record-ref.v1")).expect("record ref");
    let observation_ref = ObservationRef::new(&observation_record).expect("observation ref");
    let evidence =
        TerminalEffectEvidence::strict_decode(&schema_golden("mfm.terminal-effect-evidence.v1"))
            .expect("terminal evidence")
            .fields()
            .expect("terminal evidence fields");

    let ensure = AccessAuditEntry::new(
        &authorization_ref,
        Some(&observation_ref),
        AccessAuditStatus::Returned,
        None,
        None,
        Some(&evidence.effect_key),
        Some(&evidence.delivery_audit_ref),
    )
    .expect("returned ensure audit");
    let ensure = ensure.fields().expect("returned ensure fields");
    assert_eq!(
        ensure.delivery_audit_ref.as_ref(),
        Some(&evidence.delivery_audit_ref)
    );

    assert!(
        AccessAuditEntry::new(
            &authorization_ref,
            Some(&observation_ref),
            AccessAuditStatus::Returned,
            None,
            None,
            Some(&evidence.effect_key),
            None,
        )
        .is_err(),
        "returned ensure requires its greatest delivery-audit head"
    );
    assert!(
        AccessAuditEntry::new(
            &authorization_ref,
            Some(&observation_ref),
            AccessAuditStatus::Indeterminate,
            None,
            None,
            Some(&evidence.effect_key),
            Some(&evidence.delivery_audit_ref),
        )
        .is_err(),
        "non-return outcomes cannot claim a delivery-audit head"
    );
    AccessAuditEntry::new(
        &authorization_ref,
        Some(&observation_ref),
        AccessAuditStatus::Returned,
        None,
        None,
        None,
        None,
    )
    .expect("returned read audit has no executor delivery head");
}
