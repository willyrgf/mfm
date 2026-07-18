use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySetFor, CapabilitySpec};
use mfm_events::v1::{self as events, side_effect};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DescriptorId, DigestAlgorithm, SchemaId,
};
use mfm_program::{EffectRunner, MfmFactType, StateSpec};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_values::{ContextBoundOutput, MfmConfig, MfmValue, NonEmpty, ValidatedConfig};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    artifacts::fact_query_returned_ref_retention_refs, AdapterExecutableBinding,
    ContextOutputExtractor, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, Result, RunnerEventPayload, RunnerFactRecorded,
    RunnerIngressContext, RuntimeError, StagedArtifact, StagedRetentionRefs,
};

#[path = "runner_kit/registration.rs"]
mod registration;
pub use registration::RunnerRegistrationBuilder;

#[path = "runner_kit/artifacts.rs"]
mod artifacts;
pub use artifacts::*;
#[path = "runner_kit/builders.rs"]
mod builders;
#[cfg(test)]
pub(crate) use builders::fact_query_evidence_retention_refs;
pub use builders::*;
pub(crate) use builders::{
    RunnerClaimBinding, RunnerPreparedInvocationBinding, RunnerSideEffectBinding,
};

#[cfg(test)]
#[path = "runner_kit_tests.rs"]
mod tests;
