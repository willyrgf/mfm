use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.program.trybuild",
    name = "try_value",
    version = "1",
    schema = "mfm.program.trybuild.try_value"
)]
pub struct TryValue {
    pub amount: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.program.trybuild.public_outputs")]
pub struct TryPublicOutputs<'p, 's> {
    pub result: mfm_program::Handle<'p, 's, TryValue>,
}

#[derive(OperationOutput)]
#[mfm(schema = "mfm.program.trybuild.operation_outputs")]
pub struct TryOperationOutputs<'p, 's> {
    pub result: mfm_program::Handle<'p, 's, TryValue>,
}

#[derive(Clone, Serialize, Deserialize, MfmConfig)]
#[mfm(schema = "mfm.program.trybuild.try_config")]
pub struct TryConfig {
    pub multiplier: u64,
}

pub struct TryPureState {
    pub config: TryConfig,
}

impl mfm_program::StateSpec for TryPureState {
    type Config = TryConfig;
    type Context = mfm_program::NoContext;
    type Input = TryValue;
    type Output = TryValue;
    type Effect = mfm_capabilities::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            "mfm.program.trybuild.state",
            "try_pure_state",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.program.trybuild.state:try_pure_state"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.program.trybuild.state.try_pure_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "try_pure_state"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl mfm_program::PureState for TryPureState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> mfm_program::StateResult<Self::Output> {
        Ok(TryValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

pub struct TryMutationCap;

impl mfm_capabilities::CapabilitySpec for TryMutationCap {
    type Role = mfm_capabilities::ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<mfm_ids::CapabilityKind> {
        mfm_ids::CapabilityKind::new(
            "mfm.program.trybuild",
            "mutation",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.program.trybuild.capability:mutation"),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<mfm_ids::CapabilityVersion> {
        mfm_ids::CapabilityVersion::new("mfm.program.trybuild.capability.mutation.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mutation"
    }
}

pub struct TrySideEffectState {
    pub config: TryConfig,
}

pub struct TryCompensationState {
    pub config: TryConfig,
}

macro_rules! impl_try_side_effect_state {
    ($state:ty, $kind:literal, $version:literal, $name:literal, $digest:literal) => {
        impl mfm_program::StateSpec for $state {
            type Config = TryConfig;
            type Context = mfm_program::NoContext;
            type Input = TryValue;
            type Output = TryValue;
            type Effect = mfm_capabilities::ApplySideEffect;
            type Caps = (TryMutationCap,);

            fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
                mfm_ids::StateKind::new(
                    "mfm.program.trybuild.state",
                    $kind,
                    mfm_ids::DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::sha256_digest_bytes($digest),
                )
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
                mfm_ids::StateVersion::new($version)
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl mfm_program::SideEffectState for $state {
            type Intent = TryValue;
            type IdempotencyInput = TryValue;
            type PreparedInvocation = TryValue;
            type Submission = TryValue;
            type RecoveryEvidence = TryValue;
            type Receipt = TryValue;
            type Confirmation = TryValue;

            fn intent(
                &self,
                input: &Self::Input,
                _context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> mfm_program::StateResult<
                mfm_program::SideEffectIntent<Self::Intent, Self::IdempotencyInput>,
            > {
                let intent = TryValue {
                    amount: input.amount * self.config.multiplier,
                };
                Ok(mfm_program::SideEffectIntent::new(intent.clone(), intent))
            }

            fn output_from_receipt(
                &self,
                _input: &Self::Input,
                _prepared: &Self::PreparedInvocation,
                _submission: &Self::Submission,
                receipt: &Self::Receipt,
                _context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> mfm_program::StateResult<Self::Output> {
                Ok(receipt.clone())
            }

            fn output_from_confirmation(
                &self,
                _input: &Self::Input,
                _prepared: &Self::PreparedInvocation,
                _submission: &Self::Submission,
                _receipt: &Self::Receipt,
                confirmation: &Self::Confirmation,
                _context: &mfm_program::CertifiedContext<Self::Context>,
            ) -> mfm_program::StateResult<Self::Output> {
                Ok(confirmation.clone())
            }
        }
    };
}

impl_try_side_effect_state!(
    TrySideEffectState,
    "try_side_effect",
    "mfm.program.trybuild.state.try_side_effect.v1",
    "try_side_effect",
    b"mfm.program.trybuild.state:try_side_effect"
);
impl_try_side_effect_state!(
    TryCompensationState,
    "try_compensation",
    "mfm.program.trybuild.state.try_compensation.v1",
    "try_compensation",
    b"mfm.program.trybuild.state:try_compensation"
);

pub struct TryOperation;

impl mfm_program::Operation for TryOperation {
    type Config = TryConfig;
    type Input<'program, 'scope> = mfm_program::Handle<'program, 'scope, TryValue>;
    type Output<'program, 'scope> = TryOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<mfm_ids::OperationKind> {
        mfm_ids::OperationKind::new(
            "mfm.program.trybuild.operation",
            "try_operation",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.program.trybuild.operation:try_operation"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::OperationVersion> {
        mfm_ids::OperationVersion::new("mfm.program.trybuild.operation.try_operation.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "try_operation"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut mfm_program::OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let result = builder.state::<TryPureState, _>(
            mfm_program::StateKey::new("try-operation/state")?,
            mfm_program::NoContext,
            config.into_inner(),
            input,
        )?;
        Ok(TryOperationOutputs { result })
    }
}

pub fn seed() -> mfm_program::Result<mfm_program::CanonicalSeed<TryValue>> {
    mfm_program::CanonicalSeed::from_value(&TryValue { amount: 1 })
}
