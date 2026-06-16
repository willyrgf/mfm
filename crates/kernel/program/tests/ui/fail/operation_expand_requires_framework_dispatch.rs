#[path = "../support/types.rs"]
mod types;

use mfm_program::{Operation, OperationExpansion, OperationExpansionDispatch};
use types::{TryConfig, TryOperation, TryOperationOutputs, TryValue};

struct ForwardingOperation;

impl Operation for ForwardingOperation {
    type Config = TryConfig;
    type Input<'program, 'scope> = mfm_program::Handle<'program, 'scope, TryValue>;
    type Output<'program, 'scope> = TryOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<mfm_ids::OperationKind> {
        mfm_ids::OperationKind::new(
            "mfm.program.trybuild.operation",
            "forwarding_operation",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(
                b"mfm.program.trybuild.operation:forwarding_operation",
            ),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::OperationVersion> {
        mfm_ids::OperationVersion::new("mfm.program.trybuild.operation.forwarding_operation.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "forwarding_operation"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        dispatch: OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let operation = TryOperation;
        operation.expand(config, input, builder, dispatch)
    }
}

fn main() {}
