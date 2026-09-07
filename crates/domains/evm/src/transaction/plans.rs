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
    binding: EvmTransactionBinding,
    #[mfm(minimum_bytes = 0, maximum_bytes = 49152)]
    initcode: CanonicalBytes,
    value: EvmU256,
    gas_limit: NonZeroU64,
    max_priority_fee_per_gas: EvmU256,
    max_fee_per_gas: EvmU256,
}
impl CheckedCreatePlan {
    /// Checks all transaction parameters before context admission.
    pub fn new(
        binding: EvmTransactionBinding,
        initcode: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        let plan = Self {
            binding,
            initcode: CanonicalBytes::new(initcode),
            value,
            gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        };
        plan.validate()?;
        Ok(plan)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_transaction_parameters(
            self.initcode.as_bytes(),
            49152,
            &self.max_priority_fee_per_gas,
            &self.max_fee_per_gas,
        )
    }
    /// Returns the checked public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }
    /// Constructs the complete nonce-free command from checked parameters.
    pub fn command(&self) -> Eip1559TransactionCommand {
        Eip1559TransactionCommand {
            action: TransactionAction::Create {
                initcode: self.initcode.clone(),
            },
            binding: self.binding.clone(),
            value: self.value.clone(),
            gas_limit: self.gas_limit,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas.clone(),
            max_fee_per_gas: self.max_fee_per_gas.clone(),
        }
    }
}
checked_deserialize!(CheckedCreatePlan {
    binding: EvmTransactionBinding,
    initcode: CanonicalBytes,
    value: EvmU256,
    gas_limit: NonZeroU64,
    max_priority_fee_per_gas: EvmU256,
    max_fee_per_gas: EvmU256
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
    binding: EvmTransactionBinding,
    #[mfm(minimum_bytes = 0, maximum_bytes = 131072)]
    calldata: CanonicalBytes,
    value: EvmU256,
    gas_limit: NonZeroU64,
    max_priority_fee_per_gas: EvmU256,
    max_fee_per_gas: EvmU256,
}
impl CheckedCallPlan {
    /// Checks all transaction parameters before context admission.
    pub fn new(
        binding: EvmTransactionBinding,
        calldata: Vec<u8>,
        value: EvmU256,
        gas_limit: NonZeroU64,
        max_priority_fee_per_gas: EvmU256,
        max_fee_per_gas: EvmU256,
    ) -> Result<Self, EvmDomainError> {
        let plan = Self {
            binding,
            calldata: CanonicalBytes::new(calldata),
            value,
            gas_limit,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        };
        plan.validate()?;
        Ok(plan)
    }
    fn validate(&self) -> Result<(), EvmDomainError> {
        validate_transaction_parameters(
            self.calldata.as_bytes(),
            131072,
            &self.max_priority_fee_per_gas,
            &self.max_fee_per_gas,
        )
    }
    /// Returns the checked public transaction binding.
    pub const fn binding(&self) -> &EvmTransactionBinding {
        &self.binding
    }
    /// Constructs the complete nonce-free command from checked parameters.
    pub fn command_for(&self, target: EvmAddress) -> Eip1559TransactionCommand {
        Eip1559TransactionCommand {
            action: TransactionAction::Call {
                to: target,
                calldata: self.calldata.clone(),
            },
            binding: self.binding.clone(),
            value: self.value.clone(),
            gas_limit: self.gas_limit,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas.clone(),
            max_fee_per_gas: self.max_fee_per_gas.clone(),
        }
    }
}
checked_deserialize!(CheckedCallPlan {
    binding: EvmTransactionBinding,
    calldata: CanonicalBytes,
    value: EvmU256,
    gas_limit: NonZeroU64,
    max_priority_fee_per_gas: EvmU256,
    max_fee_per_gas: EvmU256
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
