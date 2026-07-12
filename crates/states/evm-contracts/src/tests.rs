use super::*;
use mfm_capabilities::CapabilitySet;
use mfm_evm_contract_model::{
    BlockSelector, EventName, ExpectedValue, ExternalAdoptionEvidencePolicy, FunctionName,
};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EventId, NodeId,
    RunId, SpecHash,
};
use mfm_program::StateContext;
use mfm_values::{ContextBoundOutput, MfmValue};

#[path = "tests/support.rs"]
mod support;
use self::support::*;
#[path = "tests/behavior.rs"]
mod behavior;
