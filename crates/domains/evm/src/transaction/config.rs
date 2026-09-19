use super::parameters::{FeeAmount, TransactionFees};
use super::*;

/// Checked EIP-1559 gas and fee options, with canonical full-width decimal fee encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "eip1559-options",
    version = "1",
    schema = "mfm.evm-eip1559-options"
)]
pub struct Eip1559Options {
    gas_limit: NonZeroU64,
    fees: TransactionFees,
}
impl Eip1559Options {
    /// Checks priority fee ordering; gas must already be nonzero.
    pub fn new(
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: u128,
        max_fee_per_gas: u128,
    ) -> Result<Self, EvmDomainError> {
        let value = Self {
            gas_limit,
            fees: TransactionFees {
                priority: FeeAmount(max_priority_fee_per_gas),
                maximum: FeeAmount(max_fee_per_gas),
            },
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        self.fees.validate()
    }
    /// Gas limit admitted for this transaction action.
    pub fn gas_limit(&self) -> NonZeroU64 {
        self.gas_limit
    }
    /// Maximum priority fee in its complete supported width.
    pub fn max_priority_fee_per_gas(&self) -> u128 {
        self.fees.priority.0
    }
    /// Maximum fee in its complete supported width.
    pub fn max_fee_per_gas(&self) -> u128 {
        self.fees.maximum.0
    }
}
impl_checked_deserialize!(Eip1559Options {
    gas_limit: NonZeroU64,
    fees: TransactionFees
});

/// Closed public native configuration for the maintained scalar-contract workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "contract-execution-config",
    version = "1",
    schema = "mfm.evm-contract-execution-config"
)]
pub struct EvmContractExecutionConfig {
    binding: EvmTransactionBinding,
    deployment: Eip1559Options,
    configuration: Eip1559Options,
}
impl EvmContractExecutionConfig {
    /// Retains checked public options; resource correspondence is checked by explicit binding.
    pub fn new(
        binding: EvmTransactionBinding,
        deployment: Eip1559Options,
        configuration: Eip1559Options,
    ) -> Self {
        Self {
            binding,
            deployment,
            configuration,
        }
    }
    /// Selected public authority, route and sender.
    pub fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }
    /// Options for contract creation.
    pub fn deployment(&self) -> &Eip1559Options {
        &self.deployment
    }
    /// Options for configuration calls.
    pub fn configuration(&self) -> &Eip1559Options {
        &self.configuration
    }
}
