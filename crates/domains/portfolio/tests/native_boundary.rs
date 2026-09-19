//! A second native test ABI executes the maintained Portfolio sources without EVM contracts.
//! Its fixed observation point and scripted balance are test infrastructure, not chain support.
use mfm_capabilities::{AdapterError, CallbackFailure, ReadAdapter, ReadImplementation};
use mfm_chain::balance::*;
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_ids::{ContentRef, DigestBytes, EntryPointId, RunId, StableId};
use mfm_portfolio::*;
use mfm_program::*;
use mfm_program_derive::MfmValue;
use mfm_runtime::Runtime;
use mfm_values::{InvocationDiagnostic, Object, Unsigned256};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NativeFact {
    name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Binding {
    target: BalanceTarget,
    route: NativeFact,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Query {
    account: String,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NativeValue {
    query_ref: ContentRef,
    amount: Unsigned256,
}
#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NativeOutage {
    retry_after: u32,
}
impl ClassifyError for NativeOutage {
    fn classify(&self) -> Classification {
        Classification::Retryable
    }
}
fn rejection(reason: &str) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields("test_native", "qualify", &reason, None)
}
struct Prepare<K>(PhantomData<K>);
impl<K: mfm_values::MfmValue> State for Prepare<K> {
    type Input = BalanceContext<K>;
    type Output = PreparedBalance<K>;
    type Failure = Never;
    fn state_id() -> Result<StableId> {
        Ok(StableId::new("mfm.test.native.prepare@1")?)
    }
}
impl<K: mfm_values::MfmValue> PureState for Prepare<K> {
    fn evaluate(
        context: BalanceContext<K>,
    ) -> std::result::Result<ProposedStateOutcome<PreparedBalance<K>, Never>, InvocationDiagnostic>
    {
        let ledger = context
            .active_source()
            .map_err(|cause| {
                InvocationDiagnostic::from_fields("test_native", "prepare", &cause, None)
            })?
            .target()
            .ledger()
            .clone();
        let point = ObservationPoint::new(
            ledger,
            Object::from_value(&NativeFact {
                name: "fixed-test-point".into(),
            })
            .map_err(|cause| cause.into_diagnostic("prepare"))?,
        );
        let prepared = PreparedBalance::new(context, point, DecimalScale::new(0).unwrap())
            .map_err(|cause| {
                InvocationDiagnostic::from_fields("test_native", "prepare", &cause, None)
            })?;
        Ok(ProposedStateOutcome::Success { output: prepared })
    }
}
struct Confirm<K>(PhantomData<K>);
impl<K: mfm_values::MfmValue> State for Confirm<K> {
    type Input = CandidateBalance<K>;
    type Output = BalanceContext<K>;
    type Failure = BalanceCollectionFailure;
    fn state_id() -> Result<StableId> {
        Ok(StableId::new("mfm.test.native.confirm@1")?)
    }
}
impl<K: mfm_values::MfmValue> PureState for Confirm<K> {
    fn evaluate(
        candidate: CandidateBalance<K>,
    ) -> std::result::Result<
        ProposedStateOutcome<BalanceContext<K>, BalanceCollectionFailure>,
        InvocationDiagnostic,
    > {
        match candidate.append_confirmed().map_err(|cause| {
            InvocationDiagnostic::from_fields("test_native", "confirm", &cause, None)
        })? {
            Ok(output) => Ok(ProposedStateOutcome::Success { output }),
            Err(error) => Ok(ProposedStateOutcome::Failure { failure: error }),
        }
    }
}
struct Native;
impl ReadImplementation<BalanceRead> for Native {
    type Binding = Binding;
    type NativeIntent = Query;
    type NativeEvidence = NativeValue;
    type OperationalError = NativeOutage;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.test.native.balance@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        binding: &Binding,
        intent: &ReadBalanceAt,
    ) -> std::result::Result<Query, CallbackFailure> {
        let route =
            Object::from_value(&binding.route).map_err(|cause| cause.into_diagnostic("qualify"))?;
        if intent.target() != &binding.target || intent.route_ref() != route.value_ref() {
            return Err(rejection("binding_mismatch").into());
        }
        Ok(Query {
            account: binding.target.native().decode::<NativeFact>()?.name,
        })
    }
    fn project_evidence(
        implementation: &ContentRef,
        _: &ContentRef,
        _: &Binding,
        semantic_ref: &ContentRef,
        intent: &ReadBalanceAt,
        native_ref: &ContentRef,
        _: &Query,
        native: &NativeValue,
        original: &Object,
    ) -> std::result::Result<BalanceEvidence, CallbackFailure> {
        if native_ref != &native.query_ref {
            return Err(rejection("query_mismatch").into());
        }
        Ok(BalanceEvidence::new(
            semantic_ref.clone(),
            implementation.clone(),
            original.clone(),
            BalanceOutcome::Observed {
                observed_at: intent.observed_at().clone(),
                raw_units: native.amount.clone(),
            },
        ))
    }
}
impl<K: mfm_values::MfmValue> InjectRead<ObserveBalance<K>, BalanceRead> for Native {
    type Prefix = Pure<Prepare<K>>;
    type Suffix = Pure<Confirm<K>>;
    fn surround(_: &Binding) -> Result<(Self::Prefix, Self::Suffix)> {
        Ok((Pure::default(), Pure::default()))
    }
}
impl<K> ResolveReadBinding<BalanceSourceDefinition<K>, BalanceRead> for Native {
    fn binding(source: &BalanceSourceDefinition<K>) -> Result<Binding> {
        let route = source
            .execution()
            .native()
            .decode::<NativeFact>()
            .map_err(ProgramError::Diagnostic)?;
        if source.execution().native().value_ref() != source.execution().route_ref() {
            return Err(ProgramError::Diagnostic(rejection("expected_route")));
        }
        Ok(Binding {
            target: source.source().target().clone(),
            route,
        })
    }
}
struct Resources<const COLD: bool>(Arc<AtomicUsize>);
impl<const COLD: bool> ProgramEnvironment for Resources<COLD> {
    type Sources = (PortfolioSnapshotOperation, PortfolioEnrichmentOperation);
}
impl<const COLD: bool> CapabilityFamily<BalanceRead> for Resources<COLD> {
    type Implementations = (Native,);
}
impl<K> Resolve<BalanceSourceDefinition<K>, BalanceRead> for Resources<false> {
    fn implementation(_: &BalanceSourceDefinition<K>) -> Result<StableId> {
        Ok(Native::implementation_id()?)
    }
}
impl<const COLD: bool> BindRead<BalanceRead, Native> for Resources<COLD> {
    type Adapter = Self;
    fn bind_read(&self, _: &Binding) -> std::result::Result<Self, InvocationDiagnostic> {
        Ok(Self(self.0.clone()))
    }
}
impl<const COLD: bool> ReadAdapter<Query, NativeValue, NativeOutage> for Resources<COLD> {
    fn invoke<'a>(
        &'a self,
        semantic: &'a ContentRef,
        native: &'a ContentRef,
        query: &'a Query,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<NativeValue, AdapterError<NativeOutage>>>
                + Send
                + 'a,
        >,
    > {
        assert_ne!(semantic, native);
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(NativeValue {
                query_ref: native.clone(),
                amount: Unsigned256::from_u64(if query.account == "retained" { 42 } else { 0 }),
            })
        })
    }
}

#[tokio::test]
async fn independent_native_abi_executes_both_maintained_callers_and_filters_semantic_candidates() {
    for enrichment in [false, true] {
        let ledger = LedgerIdentity::new(
            Object::from_value(&NativeFact {
                name: "other-ledger".into(),
            })
            .unwrap(),
        );
        let route = Object::from_value(&NativeFact {
            name: "public-native-route".into(),
        })
        .unwrap();
        let sources = ["retained", "empty"]
            .into_iter()
            .map(|name| {
                BalanceSource::new(
                    name.into(),
                    BalanceTarget::new(
                        ledger.clone(),
                        Object::from_value(&NativeFact { name: name.into() }).unwrap(),
                    ),
                )
                .unwrap()
            })
            .collect();
        let demand = PortfolioCollectionDemand::new(
            "independent".into(),
            BalanceRequest::new(sources, DecimalScale::new(0).unwrap()).unwrap(),
            vec![BalanceExecutionConfig::new(route.value_ref().clone(), route.clone()); 2],
        )
        .unwrap();
        let input = PortfolioSnapshotInput::new(
            PortfolioId {
                value: "portfolio".into(),
            },
            vec![demand],
            QuoteCode::Usd,
            vec![QuoteCode::Usd],
            None,
        )
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let resources = Resources::<false>(calls.clone());
        let (program, input) = if enrichment {
            let input = PortfolioEnrichmentInput::new(input, vec!["retained".into()]).unwrap();
            let program = compile(
                EntryPointId::new(PORTFOLIO_ENRICHMENT_ENTRY_POINT_ID).unwrap(),
                &PortfolioEnrichmentOperation::default(),
                &input,
                &resources,
                ProgramLimits::new(0),
            )
            .unwrap();
            (program, Object::from_value(&input).unwrap())
        } else {
            let program = compile(
                EntryPointId::new(PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID).unwrap(),
                &PortfolioSnapshotOperation::default(),
                &input,
                &resources,
                ProgramLimits::new(0),
            )
            .unwrap();
            (program, Object::from_value(&input).unwrap())
        };
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let bytes = program.canonical_bytes().to_vec();
        drop(program);
        drop(resources);
        let program = load(&bytes, &Resources::<true>(calls.clone())).unwrap();
        let runtime = Runtime::new(Arc::new(mfm_store::MemoryStore::new()));
        let run = RunId::from_digest(DigestBytes::from_array([u8::from(enrichment) + 30; 32]));
        let result = if enrichment {
            runtime
                .execute(
                    run.clone(),
                    &program,
                    &input.decode::<PortfolioEnrichmentInput>().unwrap(),
                )
                .await
                .unwrap()
        } else {
            runtime
                .execute(
                    run.clone(),
                    &program,
                    &input.decode::<PortfolioSnapshotInput>().unwrap(),
                )
                .await
                .unwrap()
        };
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        if enrichment {
            let output = result
                .success()
                .unwrap()
                .decode::<PortfolioEnrichmentOutput>()
                .unwrap();
            assert_eq!(
                output.collections()[0].demand().request().sources().len(),
                1
            );
            assert_eq!(
                output.collections()[0].demand().request().sources()[0].source_id(),
                "retained"
            );
        } else {
            let output = result
                .success()
                .unwrap()
                .decode::<PortfolioSnapshotOutput>()
                .unwrap();
            assert_eq!(
                serde_json::to_value(output.report()).unwrap()["totals_by_quote"][0]
                    ["total_value_dec"],
                "42"
            );
        }
        assert_eq!(
            runtime.read(&run, &program).await.unwrap().success(),
            result.success()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
