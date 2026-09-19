use super::*;
use crate::{AnchoredContractCallEvidence, AnchoredContractCallIntent};
use mfm_capabilities::{codec, CallbackFailure, ReadImplementation};
use mfm_chain::transaction::{
    ConfigurationValue, ConfiguredContract, ContractRead, ContractValueEvidence,
    ContractValueOutcome, LifecyclePlanning, Observe, ObservedConfiguration, ReadContractValue,
};
use mfm_program::{Identity, InjectRead, ResolveReadBinding};
use mfm_values::{InvocationDiagnostic, Object};

/// Native anchored scalar getter for the maintained EVM contract ABI.
pub struct EvmContractReadImplementation;
impl<Config: LifecyclePlanning + ?Sized> ResolveReadBinding<Config, ContractRead>
    for EvmContractReadImplementation
{
    fn binding(config: &Config) -> mfm_program::Result<EvmTransactionRoute> {
        let request = config.deployment_request();
        let selected = Self::implementation_id()?;
        if request.execution().read_implementation() != &selected {
            return Err(mfm_program::ProgramError::Diagnostic(stages::invariant(
                "scalar_read_selection",
            )));
        }
        Ok(recipes::execution_config(request)
            .map_err(|cause| mfm_program::ProgramError::Diagnostic(cause.into_diagnostic()))?
            .binding()
            .route
            .clone())
    }
}
impl InjectRead<Observe, ContractRead> for EvmContractReadImplementation {
    type Prefix = Identity<ConfiguredContract>;
    type Suffix = Identity<ObservedConfiguration>;
    fn surround(_: &EvmTransactionRoute) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((Identity::default(), Identity::default()))
    }
}
impl ReadImplementation<ContractRead> for EvmContractReadImplementation {
    type Binding = EvmTransactionRoute;
    type NativeIntent = AnchoredContractCallIntent;
    type NativeEvidence = AnchoredContractCallEvidence;
    type OperationalError = crate::EvmOperationalError;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.evm.contract-read@1")?)
    }
    fn encode_intent(
        _: &ContentRef,
        _: &ContentRef,
        binding: &EvmTransactionRoute,
        intent: &ReadContractValue,
    ) -> Result<AnchoredContractCallIntent, mfm_capabilities::CallbackFailure> {
        encode(binding, intent)
    }
    fn project_evidence(
        implementation_ref: &ContentRef,
        _: &ContentRef,
        binding: &EvmTransactionRoute,
        intent_ref: &ContentRef,
        intent: &ReadContractValue,
        native_intent_ref: &ContentRef,
        native_intent: &AnchoredContractCallIntent,
        evidence: &AnchoredContractCallEvidence,
        original: &Object,
    ) -> Result<ContractValueEvidence, mfm_capabilities::CallbackFailure> {
        if &encode(binding, intent)? != native_intent
            || evidence.intent_value_ref() != native_intent_ref
        {
            return Err(stages::invariant("project_scalar_intent").into());
        }
        evidence.validate_for(native_intent).map_err(|source| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "project_scalar_anchor",
                &source,
                None,
            )
        })?;
        let outcome = match evidence {
            AnchoredContractCallEvidence::Returned { result, .. } => {
                ContractValueOutcome::Observed {
                    observed_at: intent.at().clone(),
                    value: codec::decode(|| decode_scalar(result.return_bytes()))?,
                }
            }
            AnchoredContractCallEvidence::Rejected { .. } => ContractValueOutcome::Rejected,
            AnchoredContractCallEvidence::SafeFailure { .. } => ContractValueOutcome::SafeFailure,
            AnchoredContractCallEvidence::IntegrityBlocked { .. } => {
                ContractValueOutcome::IntegrityBlocked
            }
        };
        Ok(ContractValueEvidence::new(
            intent_ref.clone(),
            implementation_ref.clone(),
            original.clone(),
            outcome,
        ))
    }
}
fn encode(
    binding: &EvmTransactionRoute,
    intent: &ReadContractValue,
) -> Result<AnchoredContractCallIntent, CallbackFailure> {
    let ledger = codec::decode(|| {
        intent
            .target()
            .ledger()
            .native()
            .decode::<EvmChainInstance>()
    })?;
    if ledger != binding.chain_instance {
        return Err(stages::invariant("scalar_read_ledger").into());
    }
    let target = codec::decode(|| intent.target().native().decode::<EvmAddress>())?;
    let point = codec::decode(|| intent.at().native().decode::<EvmBlockPoint>())?;
    qualify_route(binding, intent.route_ref())?;
    Ok(AnchoredContractCallIntent::new(
        ledger.chain_id,
        intent.route_ref().clone(),
        crate::EvmBlockAnchor {
            number: point.number().clone(),
            hash: point.hash().clone(),
        },
        target,
        vec![0x3f, 0xa4, 0xf2, 0x45],
    )
    .map_err(|source| {
        InvocationDiagnostic::from_fields("state_internal", "encode_scalar_intent", &source, None)
    })?)
}
// Both configuration admission and native invocation qualify the caller-owned expectation here.
pub(super) fn qualify_route(
    binding: &EvmTransactionRoute,
    expected: &ContentRef,
) -> Result<(), CallbackFailure> {
    let route = codec::encode(|| {
        Object::from_value(binding)
            .map_err(|source| source.into_diagnostic("scalar_route_identity"))
    })?;
    if route.value_ref() != expected {
        return Err(InvocationDiagnostic::from_fields(
            "state_internal",
            "qualify_scalar_route",
            &serde_json::json!({"reason": "route_mismatch", "expected": expected, "actual": route.value_ref()}),
            None,
        ).into());
    }
    Ok(())
}
fn decode_scalar(bytes: &[u8]) -> Result<ConfigurationValue, InvocationDiagnostic> {
    if bytes.len() != 32 {
        return Err(InvocationDiagnostic::from_fields(
            "state_internal",
            "decode_scalar_word",
            &serde_json::json!({"expected_bytes":32,"actual_bytes":bytes.len()}),
            None,
        ));
    }
    // A 256-bit unsigned word needs at most 78 decimal digits. Each step multiplies by 256
    // and adds the next byte, retaining the complete width without a new arithmetic dependency.
    let mut digits = [0_u8; 78];
    for byte in bytes {
        let mut carry = u16::from(*byte);
        for digit in digits.iter_mut().rev() {
            let next = u16::from(*digit) * 256 + carry;
            *digit = (next % 10) as u8;
            carry = next / 10;
        }
    }
    let first = digits.iter().position(|digit| *digit != 0).unwrap_or(77);
    let decimal: String = digits[first..]
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    ConfigurationValue::new(decimal).map_err(|source| {
        InvocationDiagnostic::from_fields("state_internal", "decode_scalar_value", &source, None)
    })
}
