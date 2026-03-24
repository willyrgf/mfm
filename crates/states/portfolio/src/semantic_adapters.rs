use std::collections::BTreeMap;

use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, PrepareSourcesResponse, DEFAULT_CONTROL_SCOPE};
use mfm_evm_runtime::states::price::read_evm_oracle_unit_price;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::StateId;
use mfm_machine::io::IoProvider;
use mfm_state_common::errors::{
    state_error_with_state, state_from_io, state_unknown, state_unknown_msg,
};
use mfm_state_symbol::model::{ObservationValueSourceRef, ValuationSourceReaderConfig};
use mfm_state_wallet::model::{WalletCapabilities, WalletImplementationConfig};
use num_bigint::BigInt;
use num_traits::Zero;
use serde::Deserialize;
use serde_json::Value;

use crate::semantic::{
    AdapterId, DirectPriceSourcePayload, DirectPriceValuationPayload,
    DerivedUnitPriceValuationPayload, EvmResolvedSubjectValue, EvmRoutePolicy,
    EvmSubjectLocator, ExecutionAnchor, FixedUnitPriceValuationPayload, PinnedNetworkView,
    ResolvedSubject, ResolvedUnitPrice, RuntimeAdapter, SubjectKind, SubjectResolutionTask,
    SubjectRuntimeAdapter, SubjectRuntimeInput, ValuationRuntimeAdapter, ValuationRuntimeInput,
    ValuationTask, ViewPinTask, ViewRuntimeAdapter, ViewRuntimeInput,
};

const ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS: &str = "resolve_subject/evm_address";
const ADAPTER_PIN_VIEW_EVM: &str = "pin_view/evm";
const ADAPTER_RESOLVE_VALUATION_FIXED: &str = "resolve_valuation/fixed_unit_price";
const ADAPTER_RESOLVE_VALUATION_EVM_ORACLE: &str = "resolve_valuation/evm_oracle_direct_price";
const ADAPTER_RESOLVE_VALUATION_DERIVED: &str = "resolve_valuation/derived_unit_price";

/// Runtime adapter for compiled EVM address subjects.
#[derive(Clone, Debug, Default)]
pub struct EvmAddressSubjectRuntimeAdapter;

impl RuntimeAdapter for EvmAddressSubjectRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS.to_string()))
    }
}

#[async_trait]
impl SubjectRuntimeAdapter for EvmAddressSubjectRuntimeAdapter {
    async fn resolve_subject(
        &self,
        state_id: &StateId,
        _io: &mut dyn IoProvider,
        task: &SubjectResolutionTask,
        _input: SubjectRuntimeInput<'_>,
    ) -> Result<ResolvedSubject, StateError> {
        let locator: EvmSubjectLocator =
            decode_object_map("subject_resolution_task", &task.task_id, &task.payload)?;
        let (implementation_kind, capabilities) = match locator.implementation {
            WalletImplementationConfig::AddressOnly {} => (
                "address_only".to_string(),
                WalletCapabilities {
                    can_resolve_address: true,
                    can_sign: false,
                    can_submit: false,
                },
            ),
            WalletImplementationConfig::KeystoreEntry { .. } => {
                return Err(unsupported_wallet_impl(state_id, "keystore_entry"));
            }
            WalletImplementationConfig::NodeManagedAccount { .. } => {
                return Err(unsupported_wallet_impl(state_id, "node_managed_account"));
            }
            WalletImplementationConfig::ExternalSigner { .. } => {
                return Err(unsupported_wallet_impl(state_id, "external_signer"));
            }
        };

        Ok(ResolvedSubject {
            subject_id: task.subject_id.clone(),
            kind: SubjectKind::EvmAddress,
            value: serde_json::to_value(EvmResolvedSubjectValue {
                network_id: locator.network_id,
                address: locator.address,
                implementation_kind,
                capabilities,
            })
            .map_err(|_| {
                state_unknown(
                    "resolved_subject_serialize_failed",
                    "failed to serialize resolved subject",
                )
            })?,
        })
    }
}

/// Runtime adapter for compiled EVM execution views.
#[derive(Clone, Debug, Default)]
pub struct EvmViewRuntimeAdapter;

impl RuntimeAdapter for EvmViewRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_PIN_VIEW_EVM.to_string()))
    }
}

#[async_trait]
impl ViewRuntimeAdapter for EvmViewRuntimeAdapter {
    async fn pin_view(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ViewPinTask,
        input: ViewRuntimeInput<'_>,
    ) -> Result<PinnedNetworkView, StateError> {
        let route_policy: EvmRoutePolicy =
            decode_object_map("view_pin_task", &task.task_id, &task.payload)?;
        let prepared = input
            .prepared_sources
            .get(task.network_view_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "missing_prepared_source",
                    format!(
                        "prepared source payload `{}` was not available for view pinning",
                        task.network_view_id
                    ),
                )
            })?;
        let prepared: PrepareSourcesResponse =
            serde_json::from_value(prepared.clone()).map_err(|err| {
                state_unknown_msg(
                    "prepared_source_decode_failed",
                    format!(
                        "prepared source payload `{}` decode failed: {err}",
                        task.network_view_id
                    ),
                )
            })?;

        let control_scope = normalized_control_scope(route_policy.control_scope.as_str());
        if prepared.network_id != route_policy.network_id {
            return Err(state_unknown_msg(
                "prepared_source_network_mismatch",
                format!(
                    "prepared source network `{}` did not match route policy network `{}`",
                    prepared.network_id, route_policy.network_id
                ),
            ));
        }
        if prepared.control_scope != control_scope {
            return Err(state_unknown_msg(
                "prepared_source_scope_mismatch",
                format!(
                    "prepared source scope `{}` did not match route policy scope `{}`",
                    prepared.control_scope, control_scope
                ),
            ));
        }

        let mut client = EvmIoClient::new(state_id.clone(), io);
        let chain_id = client
            .chain_id_u64(route_policy.network_id.as_str(), control_scope)
            .await
            .map_err(state_from_io)?;
        if chain_id != route_policy.chain_id {
            return Err(state_error_with_state(
                state_id.clone(),
                "network_chain_id_mismatch",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "rpc chain_id for network `{}` did not match configured chain_id",
                    route_policy.network_id
                ),
            ));
        }

        let block_number = client
            .block_number_u64(route_policy.network_id.as_str(), control_scope)
            .await
            .map_err(state_from_io)?;

        Ok(PinnedNetworkView {
            network_view_id: task.network_view_id.clone(),
            network_id: route_policy.network_id,
            family: task.family.clone(),
            anchor: ExecutionAnchor::Evm {
                chain_id,
                block_number,
            },
        })
    }
}

/// Runtime adapter for fixed unit prices.
#[derive(Clone, Debug, Default)]
pub struct FixedUnitPriceRuntimeAdapter;

impl RuntimeAdapter for FixedUnitPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_FIXED.to_string()))
    }
}

#[async_trait]
impl ValuationRuntimeAdapter for FixedUnitPriceRuntimeAdapter {
    async fn resolve(
        &self,
        _state_id: &StateId,
        _io: &mut dyn IoProvider,
        task: &ValuationTask,
        _input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: FixedUnitPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            quote: task.quote,
            unit_price_dec: payload.unit_price_dec,
            valuation_reader_kind: "fixed_unit_price".to_string(),
            source_refs: Vec::new(),
        })
    }
}

/// Runtime adapter for direct EVM oracle prices.
#[derive(Clone, Debug, Default)]
pub struct EvmOracleDirectPriceRuntimeAdapter;

impl RuntimeAdapter for EvmOracleDirectPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_EVM_ORACLE.to_string()))
    }
}

#[async_trait]
impl ValuationRuntimeAdapter for EvmOracleDirectPriceRuntimeAdapter {
    async fn resolve(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: DirectPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        let (unit_price_dec, source_ref) =
            resolve_direct_source(state_id, io, &payload.source, input.pinned_views).await?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            quote: task.quote,
            unit_price_dec,
            valuation_reader_kind: "direct_price".to_string(),
            source_refs: vec![source_ref],
        })
    }
}

/// Runtime adapter for derived prices.
#[derive(Clone, Debug, Default)]
pub struct DerivedUnitPriceRuntimeAdapter;

impl RuntimeAdapter for DerivedUnitPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_DERIVED.to_string()))
    }
}

#[async_trait]
impl ValuationRuntimeAdapter for DerivedUnitPriceRuntimeAdapter {
    async fn resolve(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: DerivedUnitPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        let (numerator, numerator_ref) =
            resolve_direct_source(state_id, io, &payload.numerator, input.pinned_views).await?;
        let (denominator, denominator_ref) =
            resolve_direct_source(state_id, io, &payload.denominator, input.pinned_views).await?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            quote: task.quote,
            unit_price_dec: divide_decimal_strings(
                numerator.as_str(),
                denominator.as_str(),
            )?,
            valuation_reader_kind: "derived_unit_price".to_string(),
            source_refs: vec![numerator_ref, denominator_ref],
        })
    }
}

async fn resolve_direct_source(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    payload: &DirectPriceSourcePayload,
    pinned_views: &BTreeMap<String, PinnedNetworkView>,
) -> Result<(String, ObservationValueSourceRef), StateError> {
    if payload.source.network_id != payload.route_policy.network_id {
        return Err(state_unknown_msg(
            "valuation_route_network_mismatch",
            format!(
                "valuation source network `{}` did not match route policy network `{}`",
                payload.source.network_id, payload.route_policy.network_id
            ),
        ));
    }
    let pinned = pinned_evm_view_for_network(
        pinned_views,
        payload.route_policy.network_id.as_str(),
        payload.route_policy.chain_id,
    )?;
    let block_number = match pinned.anchor {
        ExecutionAnchor::Evm { block_number, .. } => block_number,
        ExecutionAnchor::Bitcoin { .. } => unreachable!("validated by pinned_evm_view_for_network"),
    };

    match &payload.source_reader {
        ValuationSourceReaderConfig::EvmOracle {
            oracle_kind,
            config,
        } => {
            let price = read_evm_oracle_unit_price(
                state_id,
                io,
                payload.route_policy.network_id.as_str(),
                normalized_control_scope(payload.route_policy.control_scope.as_str()),
                oracle_kind,
                config,
                block_number,
            )
            .await?;
            Ok((
                price.unit_price_dec,
                ObservationValueSourceRef {
                    source_id: payload.source.source_id.clone(),
                    network_id: payload.source.network_id.clone(),
                    block_number,
                },
            ))
        }
    }
}

fn pinned_evm_view_for_network<'a>(
    pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    network_id: &str,
    expected_chain_id: u64,
) -> Result<&'a PinnedNetworkView, StateError> {
    let matches = pinned_views
        .values()
        .filter(|view| view.network_id == network_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(state_unknown_msg(
            "missing_pinned_view_for_network",
            format!("missing pinned execution view for network `{network_id}`"),
        )),
        [view] => match &view.anchor {
            ExecutionAnchor::Evm { chain_id, .. } => {
                if *chain_id != expected_chain_id {
                    Err(state_unknown_msg(
                        "pinned_view_chain_id_mismatch",
                        format!(
                            "pinned execution view chain_id `{chain_id}` did not match expected chain_id `{expected_chain_id}` for network `{network_id}`",
                        ),
                    ))
                } else {
                    Ok(*view)
                }
            }
            ExecutionAnchor::Bitcoin { .. } => Err(state_unknown_msg(
                "pinned_view_family_mismatch",
                format!("pinned execution view for network `{network_id}` was not EVM"),
            )),
        },
        _ => Err(state_unknown_msg(
            "ambiguous_pinned_view_for_network",
            format!("multiple pinned execution views matched network `{network_id}`"),
        )),
    }
}

fn normalized_control_scope(control_scope: &str) -> &str {
    if control_scope.trim().is_empty() {
        DEFAULT_CONTROL_SCOPE
    } else {
        control_scope
    }
}

fn decode_object_map<T: for<'de> Deserialize<'de>>(
    entity: &'static str,
    id: &str,
    map: &BTreeMap<String, Value>,
) -> Result<T, StateError> {
    serde_json::from_value(Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
    .map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_payload_decode_failed",
            format!("{entity} `{id}` payload decode failed: {err}"),
        )
    })
}

fn unsupported_wallet_impl(state_id: &StateId, kind: &'static str) -> StateError {
    state_error_with_state(
        state_id.clone(),
        "unsupported_wallet_implementation_kind",
        ErrorCategory::Unknown,
        false,
        format!("wallet implementation kind `{kind}` is not supported in the semantic runtime"),
    )
}

fn divide_decimal_strings(numerator: &str, denominator: &str) -> Result<String, StateError> {
    let numerator = DecimalValue::parse(numerator)?;
    let denominator = DecimalValue::parse(denominator)?;
    if denominator.digits.is_zero() {
        return Err(state_unknown(
            "division_by_zero",
            "derived unit price denominator must be non-zero",
        ));
    }

    let min_scale = numerator.scale.max(denominator.scale).max(8);
    let working_scale = min_scale + 18;
    let scaled_numerator = numerator.digits * ten_pow(working_scale + denominator.scale);
    let scaled_denominator = denominator.digits * ten_pow(numerator.scale);
    let quotient = scaled_numerator / scaled_denominator;
    Ok(DecimalValue {
        digits: quotient,
        scale: working_scale,
    }
    .to_string_with_min_scale(min_scale))
}

fn ten_pow(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl DecimalValue {
    fn parse(input: &str) -> Result<Self, StateError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }

        let parts: Vec<_> = trimmed.split('.').collect();
        if parts.len() > 2
            || parts
                .iter()
                .any(|part| !part.is_empty() && !part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }

        let whole = parts[0];
        let frac = parts.get(1).copied().unwrap_or("");
        let digits = format!("{whole}{frac}");
        let digits = if digits.is_empty() {
            "0"
        } else {
            digits.as_str()
        };
        let parsed: BigInt = digits.parse().map_err(|_| {
            state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            )
        })?;
        Ok(Self {
            digits: parsed,
            scale: frac.len() as u32,
        })
    }

    fn to_string_with_min_scale(&self, min_scale: u32) -> String {
        let digits = self.digits.to_string();
        let scale = self.scale as usize;
        let mut out = if scale == 0 {
            digits
        } else if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            let split = digits.len() - scale;
            format!("{}.{}", &digits[..split], &digits[split..])
        };

        if let Some((whole, frac)) = out.split_once('.') {
            let mut frac = frac.to_string();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = format!("{whole}.{frac}");
        } else if min_scale > 0 {
            out.push('.');
            out.push_str(&"0".repeat(min_scale as usize));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::errors::{ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use serde_json::json;

    struct TestIo {
        responses: BTreeMap<String, Value>,
    }

    #[async_trait]
    impl IoProvider for TestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let kind = call
                .request
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let key = if kind == "prepare_sources" {
                format!(
                    "prepare:{}:{}",
                    call.request
                        .get("control_scope")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    call.request
                        .get("network_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                )
            } else if method == "eth_call" {
                call.request
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|params| params.first())
                    .and_then(Value::as_object)
                    .and_then(|target| target.get("data"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            } else {
                method.to_string()
            };
            let response = self.responses.get(&key).cloned().ok_or_else(|| {
                IoError::Other(ErrorInfo {
                    code: ErrorCode("missing_mock_response".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: format!("missing mock response for `{key}`"),
                    details: None,
                })
            })?;
            Ok(IoResult {
                response,
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId("artifact".to_string()))
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }
    }

    #[tokio::test]
    async fn subject_runtime_resolves_address_only_locator() {
        let adapter = EvmAddressSubjectRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::new(),
        };
        let subject = adapter
            .resolve_subject(
                &StateId::must_new("portfolio.main.resolve".to_string()),
                &mut io,
                &SubjectResolutionTask {
                    task_id: "resolve.wallet_main".to_string(),
                    subject_id: "wallet_main".to_string(),
                    kind: SubjectKind::EvmAddress,
                    adapter: AdapterId(ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS.to_string()),
                    payload: BTreeMap::from([
                        ("network_id".to_string(), json!("ethereum-mainnet")),
                        (
                            "address".to_string(),
                            json!("0x000000000000000000000000000000000000dead"),
                        ),
                        (
                            "implementation".to_string(),
                            json!({ "kind": "address_only" }),
                        ),
                    ]),
                },
                SubjectRuntimeInput {
                    prepared_sources: &BTreeMap::new(),
                },
            )
            .await
            .expect("resolved subject");

        let value: EvmResolvedSubjectValue = serde_json::from_value(subject.value).expect("typed");
        assert_eq!(value.network_id, "ethereum-mainnet");
        assert_eq!(
            value.address,
            "0x000000000000000000000000000000000000dead"
        );
    }

    #[tokio::test]
    async fn view_runtime_pins_evm_chain_and_block() {
        let adapter = EvmViewRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::from([
                ("eth_chainId".to_string(), json!("0x1")),
                ("eth_blockNumber".to_string(), json!("0x64")),
            ]),
        };
        let prepared_sources = BTreeMap::from([(
            "ethereum-mainnet".to_string(),
            serde_json::to_value(PrepareSourcesResponse {
                control_scope: "rpc.mainnet".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                pool_kind: "test".to_string(),
                available_source_ids: vec!["source-1".to_string()],
                ranked_source_ids: vec!["source-1".to_string()],
                sources: vec![],
            })
            .expect("prepared"),
        )]);

        let view = adapter
            .pin_view(
                &StateId::must_new("portfolio.main.pin".to_string()),
                &mut io,
                &ViewPinTask {
                    task_id: "pin.ethereum-mainnet".to_string(),
                    network_view_id: "ethereum-mainnet".to_string(),
                    family: crate::semantic::NetworkFamily::Evm,
                    adapter: AdapterId(ADAPTER_PIN_VIEW_EVM.to_string()),
                    payload: BTreeMap::from([
                        ("network_id".to_string(), json!("ethereum-mainnet")),
                        ("chain_id".to_string(), json!(1u64)),
                        ("control_scope".to_string(), json!("rpc.mainnet")),
                    ]),
                },
                ViewRuntimeInput {
                    prepared_sources: &prepared_sources,
                },
            )
            .await
            .expect("pinned view");

        match view.anchor {
            ExecutionAnchor::Evm {
                chain_id,
                block_number,
            } => {
                assert_eq!(chain_id, 1);
                assert_eq!(block_number, 100);
            }
            ExecutionAnchor::Bitcoin { .. } => panic!("expected evm anchor"),
        }
    }

    #[tokio::test]
    async fn valuation_runtimes_emit_unit_price_provenance() {
        let direct = EvmOracleDirectPriceRuntimeAdapter;
        let derived = DerivedUnitPriceRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::from([
                (
                    mfm_evm_runtime::states::read::encode_erc20_decimals(),
                    json!(
                        "0x0000000000000000000000000000000000000000000000000000000000000008"
                    ),
                ),
                (
                    "0xfeaf968c".to_string(),
                    json!(
                    "0x\
0000000000000000000000000000000000000000000000000000000000000001\
000000000000000000000000000000000000000000000000000000000bebc200\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001"
                    ),
                ),
            ]),
        };
        let pinned_views = BTreeMap::from([(
            "ethereum-mainnet".to_string(),
            PinnedNetworkView {
                network_view_id: "ethereum-mainnet".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                family: crate::semantic::NetworkFamily::Evm,
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            },
        )]);

        let direct_value = direct
            .resolve(
                &StateId::must_new("portfolio.main.values".to_string()),
                &mut io,
                &ValuationTask {
                    valuation_id: "eth.quote.usd".to_string(),
                    instrument_id: "eth.native.ethereum-mainnet".to_string(),
                    quote: mfm_state_symbol::model::QuoteCode::Usd,
                    adapter: AdapterId(ADAPTER_RESOLVE_VALUATION_EVM_ORACLE.to_string()),
                    payload: BTreeMap::from([
                        ("priced_symbol_id".to_string(), json!("eth.native.ethereum-mainnet")),
                        (
                            "source".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                    ]),
                },
                ValuationRuntimeInput {
                    pinned_views: &pinned_views,
                },
            )
            .await
            .expect("direct");
        assert_eq!(direct_value.valuation_reader_kind, "direct_price");
        assert_eq!(direct_value.source_refs.len(), 1);

        let derived_value = derived
            .resolve(
                &StateId::must_new("portfolio.main.values".to_string()),
                &mut io,
                &ValuationTask {
                    valuation_id: "eth.quote.btc".to_string(),
                    instrument_id: "eth.native.ethereum-mainnet".to_string(),
                    quote: mfm_state_symbol::model::QuoteCode::Btc,
                    adapter: AdapterId(ADAPTER_RESOLVE_VALUATION_DERIVED.to_string()),
                    payload: BTreeMap::from([
                        ("priced_symbol_id".to_string(), json!("eth.native.ethereum-mainnet")),
                        (
                            "numerator".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                        (
                            "denominator".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                    ]),
                },
                ValuationRuntimeInput {
                    pinned_views: &pinned_views,
                },
            )
            .await
            .expect("derived");
        assert_eq!(derived_value.valuation_reader_kind, "derived_unit_price");
        assert_eq!(derived_value.source_refs.len(), 2);
    }
}
