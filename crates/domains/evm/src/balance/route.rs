//! Retained public endpoint facts for qualification and configuration-free publication.
use super::*;
use crate::EvmEndpoint;
use mfm_capabilities::{codec, CallbackFailure};
use mfm_chain::balance::BalanceExecutionConfig;
use mfm_ids::ContentRef;
use mfm_values::ValueError;

/// Exact public balance route, including the endpoint name needed for cold publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-route",
    version = "1",
    schema = "mfm.evm-balance-route"
)]
pub struct EvmBalanceRoute {
    chain_id: NonZeroU64,
    endpoint: EvmEndpoint,
}
impl EvmBalanceRoute {
    /// Retains a checked chain identifier and public endpoint name, never a URL or credential.
    pub fn new(chain_id: NonZeroU64, endpoint: EvmEndpoint) -> Self {
        Self { chain_id, endpoint }
    }
    /// Expected native chain ID.
    pub fn chain_id(&self) -> NonZeroU64 {
        self.chain_id
    }
    /// Public endpoint identity sufficient for native configuration reconstruction.
    pub fn endpoint(&self) -> &EvmEndpoint {
        &self.endpoint
    }
    /// Independently derives the physical route from retained public facts.
    pub fn physical_target(&self) -> Result<EvmPhysicalTarget, ValueError> {
        Ok(EvmPhysicalTarget {
            chain_id: self.chain_id,
            endpoint_ref: Object::from_value(&self.endpoint)?.value_ref().clone(),
        })
    }
    /// Independently derives the route commitment from retained public facts.
    pub fn route_ref(&self) -> Result<ContentRef, ValueError> {
        Ok(Object::from_value(&self.physical_target()?)?
            .value_ref()
            .clone())
    }
    /// Decodes native facts and checks the caller-owned expectation before resource binding.
    pub fn from_execution(execution: &BalanceExecutionConfig) -> Result<Self, CallbackFailure> {
        let route = codec::decode(|| execution.native().decode::<Self>())?;
        let actual = codec::encode(|| {
            route
                .route_ref()
                .map_err(|cause| cause.into_diagnostic("balance_route_identity"))
        })?;
        if &actual != execution.route_ref() {
            #[derive(Serialize)]
            struct Mismatch<'a> {
                reason: &'static str,
                expected: &'a ContentRef,
                actual: &'a ContentRef,
            }
            return Err(InvocationDiagnostic::from_fields(
                "state_internal",
                "qualify_balance_route",
                &Mismatch {
                    reason: "route_mismatch",
                    expected: execution.route_ref(),
                    actual: &actual,
                },
                None,
            )
            .into());
        }
        Ok(route)
    }
}
