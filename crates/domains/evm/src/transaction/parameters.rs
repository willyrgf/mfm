use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
pub(super) struct FeeAmount(#[mfm(minimum_bytes = 1, maximum_bytes = 39)] pub(super) u128);

impl TryFrom<String> for FeeAmount {
    type Error = EvmDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let amount: u128 = value.parse().map_err(|_| EvmDomainError::InvalidValue)?;
        if amount.to_string() != value {
            return Err(EvmDomainError::InvalidValue);
        }
        Ok(Self(amount))
    }
}

impl From<FeeAmount> for String {
    fn from(value: FeeAmount) -> Self {
        value.0.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionFees {
    pub(super) priority: FeeAmount,
    pub(super) maximum: FeeAmount,
}

impl TransactionFees {
    fn validate(&self) -> Result<(), EvmDomainError> {
        (self.priority.0 <= self.maximum.0)
            .then_some(())
            .ok_or(EvmDomainError::InvalidValue)
    }
}
impl_checked_deserialize!(TransactionFees {
    priority: FeeAmount,
    maximum: FeeAmount,
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionParameters {
    pub(super) binding: EvmTransactionBinding,
    pub(super) value: EvmU256,
    pub(super) gas_limit: NonZeroU64,
    pub(super) fees: TransactionFees,
}

impl TransactionParameters {
    pub(super) fn new(
        binding: EvmTransactionBinding,
        value: EvmU256,
        gas_limit: NonZeroU64,
        priority: u128,
        maximum: u128,
    ) -> Result<Self, EvmDomainError> {
        let fees = TransactionFees {
            priority: FeeAmount(priority),
            maximum: FeeAmount(maximum),
        };
        fees.validate()?;
        Ok(Self {
            binding,
            value,
            gas_limit,
            fees,
        })
    }
}
