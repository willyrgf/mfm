use mfm_program::{
    Effect, Inherit, NoParams, Operation, OperationDefaults, PolicyValues, Pure, Read,
    ResolveDefaults, Stop,
};

use super::{
    Configure, ConfiguredContract, ContractDeploymentReport, ContractRead, Deploy,
    DeployedContract, DeploymentRequest, Observe, ObservedConfiguration, Report, TransactionEffect,
    Validate, ValidatedConfiguration,
};

/// Borrowed, already-admitted lifecycle planning facts, independent of future execution values.
pub trait LifecyclePlanning {
    /// Original request supplying implementation selectors, public options and recovery allowances.
    fn deployment_request(&self) -> &DeploymentRequest;
}
impl LifecyclePlanning for DeploymentRequest {
    fn deployment_request(&self) -> &DeploymentRequest {
        self
    }
}
impl LifecyclePlanning for DeployedContract {
    fn deployment_request(&self) -> &DeploymentRequest {
        self.request()
    }
}
impl LifecyclePlanning for ConfiguredContract {
    fn deployment_request(&self) -> &DeploymentRequest {
        self.configured().request()
    }
}
impl LifecyclePlanning for ObservedConfiguration {
    fn deployment_request(&self) -> &DeploymentRequest {
        self.configured().deployment_request()
    }
}
impl LifecyclePlanning for ValidatedConfiguration {
    fn deployment_request(&self) -> &DeploymentRequest {
        self.observed().deployment_request()
    }
}
impl LifecyclePlanning for ContractDeploymentReport {
    fn deployment_request(&self) -> &DeploymentRequest {
        self.request()
    }
}

/// Baseline lifecycle policy: Stop, no recovery targets, and explicit zero allowance fallback.
pub struct LifecycleDefaults;
impl OperationDefaults for LifecycleDefaults {
    type Handler = Stop;
    type Targets = ();
}
impl<C: LifecyclePlanning + ?Sized> ResolveDefaults<C> for LifecycleDefaults {
    fn resolve(config: &C) -> mfm_program::Result<PolicyValues<Stop>> {
        let request = config.deployment_request();
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(request.retry_allowance().unwrap_or(0)),
            restarts: Some(request.restart_allowance().unwrap_or(0)),
        })
    }
}

/// Maintained deployment, configuration, observation, validation and reporting sequence.
pub type ContractDeploymentLifecycle = Operation<
    (
        Effect<Deploy, TransactionEffect<DeploymentRequest>>,
        Effect<Configure, TransactionEffect<DeployedContract>>,
        Read<Observe, ContractRead>,
        Pure<Validate>,
        Pure<Report>,
    ),
    LifecycleDefaults,
>;

/// Reusable configuration/observation child, inheriting its enclosing policy unchanged.
pub type ConfigureAndObserve = Operation<
    (
        Effect<Configure, TransactionEffect<DeployedContract>>,
        Read<Observe, ContractRead>,
    ),
    Inherit,
>;
