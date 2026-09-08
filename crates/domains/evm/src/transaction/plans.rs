use super::*;

/// Checked transaction parameters whose creation action is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "checked-create-plan",
    version = "1",
    schema = "mfm.evm-checked-create-plan"
)]
pub struct CheckedCreatePlan {
    parameters: TransactionParameters,
    #[mfm(minimum_bytes = 0, maximum_bytes = 49152)]
    initcode: CanonicalBytes,
}
impl CheckedCreatePlan {
    /// Checks all transaction parameters before context admission.
    pub fn new(
        binding: EvmTransactionBinding,
        initcode: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: u128,
        max_fee_per_gas: u128,
    ) -> Result<Self, EvmDomainError> {
        let plan = Self {
            parameters: TransactionParameters::new(
                binding,
                value,
                gas_limit,
                max_priority_fee_per_gas,
                max_fee_per_gas,
            )?,
            initcode: CanonicalBytes::new(initcode),
        };
        plan.validate()?;
        Ok(plan)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_input_bytes(self.initcode.as_bytes(), MAX_EVM_INITCODE_BYTES)
    }
    /// Returns the checked public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.parameters.binding
    }
    /// Constructs the complete nonce-free command from checked parameters.
    pub fn command(&self) -> Eip1559TransactionCommand {
        Eip1559TransactionCommand {
            action: TransactionAction::Create {
                initcode: self.initcode.clone(),
            },
            parameters: self.parameters.clone(),
        }
    }
}
impl_checked_deserialize!(CheckedCreatePlan {
    parameters: TransactionParameters,
    initcode: CanonicalBytes,
});

/// Checked transaction parameters whose call target is supplied by a later fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "checked-call-plan",
    version = "1",
    schema = "mfm.evm-checked-call-plan"
)]
pub struct CheckedCallPlan {
    parameters: TransactionParameters,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    calldata: CanonicalBytes,
}
impl CheckedCallPlan {
    /// Checks all transaction parameters before context admission.
    pub fn new(
        binding: EvmTransactionBinding,
        calldata: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: u128,
        max_fee_per_gas: u128,
    ) -> Result<Self, EvmDomainError> {
        let plan = Self {
            parameters: TransactionParameters::new(
                binding,
                value,
                gas_limit,
                max_priority_fee_per_gas,
                max_fee_per_gas,
            )?,
            calldata: CanonicalBytes::new(calldata),
        };
        plan.validate()?;
        Ok(plan)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_input_bytes(self.calldata.as_bytes(), MAX_EVM_CALLDATA_BYTES)
    }
    /// Returns the checked public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.parameters.binding
    }
    /// Constructs the complete nonce-free command from checked parameters.
    pub fn command_for(&self, target: EvmAddress) -> Eip1559TransactionCommand {
        Eip1559TransactionCommand {
            action: TransactionAction::Call {
                to: target,
                calldata: self.calldata.clone(),
            },
            parameters: self.parameters.clone(),
        }
    }
}
impl_checked_deserialize!(CheckedCallPlan {
    parameters: TransactionParameters,
    calldata: CanonicalBytes,
});

/// Checked ordinary call plan with a required target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "checked-target-call-plan",
    version = "1",
    schema = "mfm.evm-checked-target-call-plan"
)]
pub struct CheckedTargetCallPlan {
    plan: CheckedCallPlan,
    target: EvmAddress,
}
impl CheckedTargetCallPlan {
    /// Combines a checked target-free call plan and required target.
    pub const fn new(plan: CheckedCallPlan, target: EvmAddress) -> Self {
        Self { plan, target }
    }
    /// Returns the original checked call parameters.
    pub const fn plan(&self) -> &CheckedCallPlan {
        &self.plan
    }
    /// Returns the required target.
    pub const fn target(&self) -> &EvmAddress {
        &self.target
    }
    /// Constructs the complete nonce-free call command.
    pub fn command(&self) -> Eip1559TransactionCommand {
        self.plan.command_for(self.target.clone())
    }
}
