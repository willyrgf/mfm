use super::*;
use crate::{make_run_services, ErrorClass, RunLaunchRequest, RunModeStatus, RunServices};
use alloy_primitives::keccak256;
use mfm_adapters_evm_contracts::{
    ensure_prepared_invocation_public, EvmContractReadRuntime, EvmContractRuntime,
    EvmContractRuntimeFactory, PreparedContractInvocation,
};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_certify::CertificationRegistry;
use mfm_core::crypto::EthereumPrivateKey;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector,
    EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
    EvmCapabilityFuture, EvmChainIdentityProvider, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest, EvmCodeReadResponse,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmLogsReadResponse, EvmNetworkBinding, EvmNetworkId, EvmNonceOccupancy,
    EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest, EvmNonceOccupancyReadResponse,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider,
    EvmReceiptReadRequest, EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef,
    EvmTransactionSubmitProvider, EvmTransactionSubmitRequest, EvmTransactionSubmitResponse,
    RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    ContextBoundValidationReport, ContractArtifactConfig, LifecycleArtifactEvidenceRef,
};
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm};
use mfm_runtime::{CertifiedRuntimeSpec, ErasedRunnerRegistry};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture, SigningProvider,
    SigningRequest, SigningResult,
};
use mfm_spec::v1 as spec;
use mfm_store::v1::{
    self as store, ExecutionClaimStatus, ExecutionClaimStore, RetainedArtifactReadProvider,
    RunEventStore,
};
use mfm_values::{MfmConfig, MfmValue};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TEST_SIGNER_HEX: &str = "4c0883a69102937d6231471b5dbb6204fe512961708279c2f802d6a8ebf2d3a4";

type ContractRunStore = store::AsyncInMemoryRunStore;
type ContractRunServices = RunServices<ContractRunStore, ContractArtifactOverlay>;

#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;

#[path = "test_support/launch.rs"]
mod launch_support;
use self::launch_support::*;
#[path = "test_support/artifacts.rs"]
mod artifacts;
use self::artifacts::*;
#[path = "test_support/provider.rs"]
mod provider;
use self::provider::*;
#[path = "test_support/config.rs"]
mod config;
use self::config::*;
#[path = "test_support/output.rs"]
mod output;
use self::output::*;
