use super::*;
use mfm_capabilities::{codec, EffectCapabilityContract, EffectImplementation};
use mfm_chain::transaction::{
    PreparedTransaction, TransactionEffect, TransactionEvidence, TransactionResult,
};
use mfm_chain::{LedgerIdentity, ObservationPoint, TransactionIdentity};
use mfm_program::{EffectSelection, Identity, InjectEffect, ResolvedEffect};
use mfm_values::{InvocationDiagnostic, Object};

/// Exact native block identity retained in shared observation envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "block-point",
    version = "1",
    schema = "mfm.evm-block-point"
)]
pub struct EvmBlockPoint {
    number: EvmU256,
    hash: EvmHash,
}
impl EvmBlockPoint {
    /// Retains the complete checked native number and hash.
    pub fn new(number: EvmU256, hash: EvmHash) -> Self {
        Self { number, hash }
    }
    /// Native block number, without narrowing its range.
    pub fn number(&self) -> &EvmU256 {
        &self.number
    }
    /// Exact block hash.
    pub fn hash(&self) -> &EvmHash {
        &self.hash
    }
}

/// Native EVM transaction execution shared by supported semantic request specializations.
pub struct EvmTransactionImplementation;
impl<R: EvmTransactionRecipe> EffectImplementation<TransactionEffect<R>>
    for EvmTransactionImplementation
{
    type Binding = EvmTransactionBinding;
    type NativeCommand = PreparedEvmTransaction;
    type NativeEvidence = EvmTransactionSettlement;
    type OperationalError = crate::EvmTransactionOperationalError;
    fn implementation_id() -> mfm_capabilities::Result<StableId> {
        Ok(StableId::new("mfm.evm.transaction@1")?)
    }
    fn decode_command(
        implementation_ref: &ContentRef,
        binding_ref: &ContentRef,
        binding: &EvmTransactionBinding,
        _: &ContentRef,
        command: &PreparedTransaction<R>,
    ) -> Result<(ContentRef, PreparedEvmTransaction), mfm_capabilities::CallbackFailure> {
        if command.implementation_ref() != implementation_ref
            || command.binding_ref() != binding_ref
        {
            return Err(stages::invariant("decode_transaction_identity").into());
        }
        let native = codec::decode(|| {
            command
                .native()
                .admit(
                    &PreparedEvmTransaction::schema_descriptor()
                        .map_err(|error| error.into_diagnostic("native_command_schema"))?,
                )
                .map_err(|error| error.into_diagnostic("native_command_admission"))?;
            command.native().decode::<PreparedEvmTransaction>()
        })?;
        if native.reserved().command().binding() != binding
            || &command.request().command()? != native.reserved().command()
        {
            return Err(stages::invariant("decode_transaction_command").into());
        }
        Ok((command.native().value_ref().clone(), native))
    }
    fn project_evidence(
        _: &ContentRef,
        _: &ContentRef,
        binding: &EvmTransactionBinding,
        effect_id: &EffectId,
        command_ref: &ContentRef,
        command: &PreparedTransaction<R>,
        native: &PreparedEvmTransaction,
        evidence: &EvmTransactionSettlement,
        original: &Object,
    ) -> Result<TransactionEvidence<R::Applied>, mfm_capabilities::CallbackFailure> {
        evidence.qualify(effect_id, native)?;
        let ledger = LedgerIdentity::new(codec::encode(|| {
            Object::from_value(&binding.route.chain_instance)
                .map_err(|error| error.into_diagnostic("project_transaction_ledger"))
        })?);
        let transaction = TransactionIdentity::new(
            ledger.clone(),
            codec::encode(|| {
                Object::from_value(evidence.transaction_hash())
                    .map_err(|error| error.into_diagnostic("project_transaction_hash"))
            })?,
        );
        let point = EvmBlockPoint::new(
            evidence.block_anchor().number.clone(),
            evidence.block_anchor().hash.clone(),
        );
        let observed_at = ObservationPoint::new(
            ledger,
            codec::encode(|| {
                Object::from_value(&point)
                    .map_err(|error| error.into_diagnostic("project_transaction_point"))
            })?,
        );
        let result = match evidence.outcome() {
            EvmTransactionOutcome::Reverted => TransactionResult::Rejected { reason: None },
            EvmTransactionOutcome::Created { .. } | EvmTransactionOutcome::Called => {
                TransactionResult::Applied {
                    output: command.request().applied(evidence)?,
                }
            }
        };
        Ok(TransactionEvidence::new(
            effect_id.clone(),
            command_ref.clone(),
            command.implementation_ref().clone(),
            transaction,
            observed_at,
            original.clone(),
            result,
        )
        .map_err(|error| {
            InvocationDiagnostic::from_fields(
                "state_internal",
                "project_transaction_evidence",
                &error,
                None,
            )
        })?)
    }
}

/// Identity native implementation for reservation under the selected transaction binding.
pub struct EvmNonceReservationImplementation;
/// Identity native implementation for preparation under the selected transaction binding.
pub struct EvmTransactionPreparationImplementation;

macro_rules! support {
    ($implementation:ident, $capability:ident, $command:ty, $evidence:ty, $id:literal, $binding:expr) => {
        impl EffectImplementation<$capability> for $implementation {
            type Binding = EvmTransactionBinding;
            type NativeCommand = $command;
            type NativeEvidence = $evidence;
            type OperationalError = crate::EvmTransactionOperationalError;
            fn implementation_id() -> mfm_capabilities::Result<StableId> {
                Ok(StableId::new($id)?)
            }
            fn decode_command(
                _: &ContentRef,
                _: &ContentRef,
                binding: &EvmTransactionBinding,
                command_ref: &ContentRef,
                command: &$command,
            ) -> Result<(ContentRef, $command), mfm_capabilities::CallbackFailure> {
                if ($binding)(command) != binding {
                    return Err(stages::invariant("decode_support_binding").into());
                }
                Ok((command_ref.clone(), command.clone()))
            }
            fn project_evidence(
                _: &ContentRef,
                _: &ContentRef,
                _: &EvmTransactionBinding,
                effect_id: &EffectId,
                command_ref: &ContentRef,
                command: &$command,
                _: &$command,
                evidence: &$evidence,
                original: &Object,
            ) -> Result<$evidence, mfm_capabilities::CallbackFailure> {
                <$capability as EffectCapabilityContract>::bind_evidence(
                    effect_id,
                    command_ref,
                    command,
                    original.value_ref(),
                    evidence,
                )?;
                Ok(evidence.clone())
            }
        }
        impl<S> InjectEffect<S, $capability> for $implementation
        where
            S: EffectSelection<
                $capability,
                ExpandedInput = <S as mfm_program::State>::Input,
                ExpandedOutput = <S as mfm_program::State>::Output,
            >,
        {
            type Prefix = Identity<S::Input>;
            type Suffix = Identity<S::Output>;
            fn surround(
                _: &EvmTransactionBinding,
            ) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
                Ok((Identity::default(), Identity::default()))
            }
        }
    };
}
support!(
    EvmNonceReservationImplementation,
    EvmNonceReservationEffect,
    Eip1559TransactionCommand,
    Reservation,
    "mfm.evm.reserve-nonce@1",
    Eip1559TransactionCommand::binding
);
support!(
    EvmTransactionPreparationImplementation,
    EvmTransactionPreparationEffect,
    ReservedEvmTransaction,
    PreparedEvmTransactionEvidence,
    "mfm.evm.prepare-transaction@1",
    reserved_binding
);

impl<R, S> InjectEffect<S, TransactionEffect<R>> for EvmTransactionImplementation
where
    R: EvmTransactionRecipe,
    S: EffectSelection<
            TransactionEffect<R>,
            ExpandedInput = R,
            ExpandedOutput = <S as mfm_program::State>::Output,
        > + mfm_program::State<Input = PreparedTransaction<R>>,
{
    type Prefix = (
        ResolvedEffect<
            ReserveEvmNonce<R>,
            EvmNonceReservationEffect,
            EvmNonceReservationImplementation,
        >,
        ResolvedEffect<
            PrepareEvmTransaction<R>,
            EvmTransactionPreparationEffect,
            EvmTransactionPreparationImplementation,
        >,
    );
    type Suffix = Identity<S::Output>;
    fn surround(
        binding: &EvmTransactionBinding,
    ) -> mfm_program::Result<(Self::Prefix, Self::Suffix)> {
        Ok((
            (
                ResolvedEffect::new(binding.clone()),
                ResolvedEffect::new(binding.clone()),
            ),
            Identity::default(),
        ))
    }
}

fn reserved_binding(command: &ReservedEvmTransaction) -> &EvmTransactionBinding {
    command.command().binding()
}
