use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mfm_capabilities::{CapabilitySpec, ReadExternalRole};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, MfmFactType as _, NoContext,
    PublicOutputKey, ReadState, RootBuilder, ScopeKey, StateKey, StateRegistryBuilder, StateResult,
    StateSpec, TypedProgramLaunchPlan,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::ExecutionClaimStore as _;
use serde::{Deserialize, Serialize};

#[path = "tests/fact_support.rs"]
mod fact_support;
#[path = "tests/registry.rs"]
mod registry;
use self::fact_support::*;
#[path = "tests/artifact_overrides.rs"]
mod artifact_overrides;
use self::artifact_overrides::*;
#[path = "tests/behavior.rs"]
mod behavior;
#[path = "tests/evm_contract_workflow.rs"]
mod evm_contract_workflow;
#[path = "tests/evm_transaction.rs"]
mod evm_transaction;
#[path = "tests/evm_validation.rs"]
mod evm_validation;
