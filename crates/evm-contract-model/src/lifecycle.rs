use super::*;

/// Lifecycle stage required or produced by an import/transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "lifecycle-stage",
    schema = "mfm.evm.contract.value.lifecycle_stage"
)]
pub enum ContractLifecycleStage {
    /// Deployed contract instance stage.
    Deployed,
    /// Configured contract instance stage.
    Configured,
}

/// Source cell or public output selected for an MFM-run import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "source-cell-or-output-ref",
    schema = "mfm.evm.contract.value.source_cell_or_output_ref"
)]
pub enum SourceCellOrOutputRef {
    /// Source cell id inside the source run authority.
    Cell {
        /// Source cell id.
        cell_id: LifecycleCellIdRef,
    },
    /// Source terminal public-output binding key.
    PublicOutput {
        /// Source public output key.
        output_key: ArtifactPort,
    },
}

/// Certified policy for accepting a source-run context during import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "accepted-context-policy",
    schema = "mfm.evm.contract.value.accepted_context_policy"
)]
pub enum AcceptedContextPolicy {
    /// Source context must equal the importing context.
    ExactContext {},
    /// Source context must be one of these explicit context refs.
    AcceptedContextRefs {
        /// Accepted source context refs.
        context_refs: Vec<ContextRefValue>,
    },
}

impl Default for AcceptedContextPolicy {
    fn default() -> Self {
        Self::ExactContext {}
    }
}

/// Import request for a lifecycle value produced by another verified MFM run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "import-from-mfm-run",
    schema = "mfm.evm.contract.value.import_from_mfm_run"
)]
pub struct ImportFromMfmRun {
    /// Source run id.
    pub source_run_id: LifecycleRunIdRef,
    /// Source certified spec hash.
    pub source_spec_hash: LifecycleSpecHashRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Required source lifecycle stage.
    pub required_stage: ContractLifecycleStage,
    /// Certified context acceptance policy.
    pub accepted_context_policy: AcceptedContextPolicy,
}

impl<'de> Deserialize<'de> for ImportFromMfmRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawImportFromMfmRun {
            source_run_id: LifecycleRunIdRef,
            source_spec_hash: LifecycleSpecHashRef,
            source_cell_or_output_id: SourceCellOrOutputRef,
            source_value_digest: ContractProfileDigestRef,
            source_context_ref: ContextRefValue,
            required_stage: ContractLifecycleStage,
            #[serde(default)]
            accepted_context_policy: AcceptedContextPolicy,
        }

        let raw = RawImportFromMfmRun::deserialize(deserializer)?;
        Ok(Self {
            source_run_id: raw.source_run_id,
            source_spec_hash: raw.source_spec_hash,
            source_cell_or_output_id: raw.source_cell_or_output_id,
            source_value_digest: raw.source_value_digest,
            source_context_ref: raw.source_context_ref,
            required_stage: raw.required_stage,
            accepted_context_policy: raw.accepted_context_policy,
        })
    }
}

/// Replayable evidence for importing a lifecycle value from another MFM run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "import-from-mfm-run-evidence",
    schema = "mfm.evm.contract.value.import_from_mfm_run_evidence"
)]
pub struct ImportFromMfmRunEvidence {
    /// Source certified spec hash.
    pub source_spec_hash: LifecycleSpecHashRef,
    /// Retained source typed-spec artifact verified against the source stream admission event.
    pub source_spec_artifact_ref: LifecycleArtifactEvidenceRef,
    /// Retained source typed-spec certificate verified against the compiled registry.
    pub source_spec_certificate_ref: LifecycleArtifactEvidenceRef,
    /// Retained canonical committed source run stream.
    pub source_run_stream_ref: LifecycleArtifactEvidenceRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value schema id.
    pub source_cell_schema_id: ArtifactEvidenceSchemaId,
    /// Source value semantic type id.
    pub source_cell_semantic_type_id: ArtifactEvidenceSemanticTypeId,
    /// Source producer descriptor id.
    pub source_producer_descriptor_id: LifecycleDescriptorIdRef,
    /// Source lifecycle stage.
    pub source_stage: ContractLifecycleStage,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Source context descriptor id.
    pub source_context_descriptor_id: LifecycleContextDescriptorIdRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
    /// Retained source value artifact or inline canonical value evidence.
    pub source_value_artifact_ref_or_inline_canonical_value: LifecycleArtifactEvidenceRef,
    /// Source terminal cell or output event id.
    pub source_terminal_cell_or_output_event_ref: LifecycleEventIdRef,
    /// Certified import policy digest.
    pub import_policy_digest: ContractProfileDigestRef,
}

/// One terminal source-run cell or output event derived from a committed source stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "source-run-terminal-event",
    schema = "mfm.evm.contract.value.source_run_terminal_event"
)]
pub struct SourceRunTerminalEvent {
    /// Source terminal cell or output event id.
    pub source_terminal_cell_or_output_event_ref: LifecycleEventIdRef,
    /// Source cell or output id.
    pub source_cell_or_output_id: SourceCellOrOutputRef,
    /// Source value schema id.
    pub source_cell_schema_id: ArtifactEvidenceSchemaId,
    /// Source value semantic type id.
    pub source_cell_semantic_type_id: ArtifactEvidenceSemanticTypeId,
    /// Source producer descriptor id.
    pub source_producer_descriptor_id: LifecycleDescriptorIdRef,
    /// Source lifecycle stage.
    pub source_stage: ContractLifecycleStage,
    /// Source context ref.
    pub source_context_ref: ContextRefValue,
    /// Source context descriptor id.
    pub source_context_descriptor_id: LifecycleContextDescriptorIdRef,
    /// Source value digest.
    pub source_value_digest: ContractProfileDigestRef,
}

/// Certified evidence policy for adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-adoption-evidence-policy",
    schema = "mfm.evm.contract.value.external_adoption_evidence_policy"
)]
pub struct ExternalAdoptionEvidencePolicy {
    /// Whether code must be observed at the adopted address.
    pub require_code: bool,
    /// Optional expected deployed code hash.
    pub expected_code_hash: Option<EvmCodeHash>,
    /// Optional observation block anchor.
    pub block_anchor: Option<BlockSelector>,
    /// Initial read assertions required before adoption.
    pub initial_read_assertions: Vec<ReadAssertionConfig>,
    /// Initial event assertions required before adoption.
    pub initial_event_assertions: Vec<EventAssertionConfig>,
    /// Whether an unverified configured claim is allowed.
    pub allow_external_claimed_configured: bool,
}

fn default_require_code() -> bool {
    true
}

impl Default for ExternalAdoptionEvidencePolicy {
    fn default() -> Self {
        Self {
            require_code: true,
            expected_code_hash: None,
            block_anchor: None,
            initial_read_assertions: Vec::new(),
            initial_event_assertions: Vec::new(),
            allow_external_claimed_configured: false,
        }
    }
}

impl<'de> Deserialize<'de> for ExternalAdoptionEvidencePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawExternalAdoptionEvidencePolicy {
            #[serde(default = "default_require_code")]
            require_code: bool,
            #[serde(default)]
            expected_code_hash: Option<EvmCodeHash>,
            #[serde(default)]
            block_anchor: Option<BlockSelector>,
            #[serde(default)]
            initial_read_assertions: Vec<ReadAssertionConfig>,
            #[serde(default)]
            initial_event_assertions: Vec<EventAssertionConfig>,
            #[serde(default)]
            allow_external_claimed_configured: bool,
        }

        let raw = RawExternalAdoptionEvidencePolicy::deserialize(deserializer)?;
        Ok(Self {
            require_code: raw.require_code,
            expected_code_hash: raw.expected_code_hash,
            block_anchor: raw.block_anchor,
            initial_read_assertions: raw.initial_read_assertions,
            initial_event_assertions: raw.initial_event_assertions,
            allow_external_claimed_configured: raw.allow_external_claimed_configured,
        })
    }
}

/// Request to adopt an external EVM address under a certified lifecycle context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "adopt-external-address",
    schema = "mfm.evm.contract.value.adopt_external_address"
)]
pub struct AdoptExternalAddress {
    /// Normalized address being adopted.
    pub address: ContractAddress,
    /// Redaction-safe provenance label.
    pub provenance_label: ProvenanceLabel,
    /// Certified evidence policy.
    pub evidence_policy: ExternalAdoptionEvidencePolicy,
}

impl<'de> Deserialize<'de> for AdoptExternalAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawAdoptExternalAddress {
            address: ContractAddress,
            provenance_label: ProvenanceLabel,
            #[serde(default)]
            evidence_policy: ExternalAdoptionEvidencePolicy,
        }

        let raw = RawAdoptExternalAddress::deserialize(deserializer)?;
        Ok(Self {
            address: raw.address,
            provenance_label: raw.provenance_label,
            evidence_policy: raw.evidence_policy,
        })
    }
}

/// Redacted EVM source evidence captured with an external adoption read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-evm-source-evidence",
    schema = "mfm.evm.contract.value.external_evm_source_evidence"
)]
pub struct ExternalEvmSourceEvidence {
    /// Semantic network id captured from the bound provider evidence.
    pub network_id: String,
    /// Expected EVM chain id captured from the certified network context.
    pub expected_chain_id: u64,
    /// Observed EVM chain id reported by the provider.
    pub observed_chain_id: u64,
    /// Redacted provider source reference.
    pub source_ref: String,
    /// Redacted provider source policy id.
    pub policy_id: String,
}

/// Replayable code-read evidence captured while adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-code-read-evidence",
    schema = "mfm.evm.contract.value.external_code_read_evidence"
)]
pub struct ExternalCodeReadEvidence {
    /// Address whose deployed bytecode was read.
    pub address: ContractAddress,
    /// Block selector used for the code read.
    pub block: BlockSelector,
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Observed bytecode hash.
    pub observed_code_hash: EvmCodeHash,
    /// Observed bytecode length.
    pub observed_code_byte_len: u64,
}

/// Replayable read-assertion evidence captured during external configured adoption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-read-assertion-evidence",
    schema = "mfm.evm.contract.value.external_read_assertion_evidence"
)]
pub struct ExternalReadAssertionEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationReadResult,
}

/// Replayable event-assertion evidence captured during external configured adoption.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-event-assertion-evidence",
    schema = "mfm.evm.contract.value.external_event_assertion_evidence"
)]
pub struct ExternalEventAssertionEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ExternalEvmSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationEventResult,
}

/// Replayable evidence captured while adopting an external EVM address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "external-adoption-evidence",
    schema = "mfm.evm.contract.value.external_adoption_evidence"
)]
pub struct ExternalAdoptionEvidence {
    /// Certified evidence policy digest.
    pub evidence_policy_digest: ContractProfileDigestRef,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Lifecycle stage admitted by this external adoption.
    pub resource_stage: ContractLifecycleStage,
    /// Observed EVM chain id.
    pub observed_chain_id: u64,
    /// Replayable code-read evidence when the policy required a code observation.
    pub code_read_evidence: Option<ExternalCodeReadEvidence>,
    /// Replayable read assertion evidence.
    pub read_assertion_evidence: Vec<ExternalReadAssertionEvidence>,
    /// Replayable event assertion evidence.
    pub event_assertion_evidence: Vec<ExternalEventAssertionEvidence>,
}

/// Claim describing why an instance is considered configured.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configuration-claim",
    schema = "mfm.evm.contract.value.configuration_claim"
)]
pub enum ConfigurationClaim {
    /// Configuration was performed by an MFM configure transition.
    MfmConfigured {
        /// Configure node id.
        configure_node: LifecycleNodeIdRef,
        /// Digest of the configure action.
        configure_action_digest: ContractProfileDigestRef,
        /// Configuration call evidence refs.
        call_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
        /// Confirmation read/event evidence refs.
        confirmation_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    },
    /// Configuration provenance was imported from another verified MFM run.
    ImportedMfmConfigured {
        /// Source run id.
        source_run_id: LifecycleRunIdRef,
        /// Source spec hash.
        source_spec_hash: LifecycleSpecHashRef,
        /// Source cell or output id.
        source_cell_or_output_id: SourceCellOrOutputRef,
        /// Source value digest.
        source_value_digest: ContractProfileDigestRef,
        /// Source context ref.
        source_context_ref: ContextRefValue,
    },
    /// External adoption observed configured-state evidence.
    ExternalObservedConfigured {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest.
        evidence_policy_digest: ContractProfileDigestRef,
        /// Digest of the inline external adoption evidence that supports the observation.
        external_adoption_evidence_digest: ContractProfileDigestRef,
    },
    /// External adoption made an explicitly unverified configured claim.
    ExternalClaimedConfigured {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest permitting this claim.
        evidence_policy_digest: ContractProfileDigestRef,
    },
}

impl ConfigurationClaim {
    /// Returns true when this claim proves MFM configuration provenance.
    pub const fn proves_mfm_configuration(&self) -> bool {
        matches!(
            self,
            Self::MfmConfigured { .. } | Self::ImportedMfmConfigured { .. }
        )
    }
}

/// Provenance for a deployed contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-provenance",
    schema = "mfm.evm.contract.value.deploy_provenance"
)]
pub enum DeployProvenance {
    /// Contract was deployed by an MFM deploy transition.
    MfmDeploy {
        /// Deployment transaction hash.
        deploy_tx_hash: String,
    },
    /// Deployed instance was imported from another verified MFM run.
    ImportedMfmRun {
        /// Imported source context ref.
        source_context_ref: ContextRefValue,
        /// Imported source value digest.
        source_value_digest: ContractProfileDigestRef,
        /// Certified import policy digest.
        import_policy_digest: ContractProfileDigestRef,
    },
    /// Deployed instance was externally adopted.
    ExternalAdoption {
        /// Redaction-safe provenance label.
        provenance_label: ProvenanceLabel,
        /// Certified evidence policy digest.
        evidence_policy_digest: ContractProfileDigestRef,
    },
}

/// Context-bound deployed contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deployed-contract-instance",
    schema = "mfm.evm.contract.value.deployed_contract_instance"
)]
pub struct DeployedContractInstance {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Deployment or import provenance.
    pub deploy_provenance: DeployProvenance,
    /// Deployment or import evidence refs.
    pub deploy_evidence: Vec<LifecycleArtifactEvidenceRef>,
    /// Replayable external-adoption evidence when this instance was adopted externally.
    pub external_adoption_evidence: Option<ExternalAdoptionEvidence>,
    /// Optional block number that confirmed deployment or adoption.
    pub deployed_block_number: Option<u64>,
}

/// Configured instance lineage back to a same-context deployed address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-from",
    schema = "mfm.evm.contract.value.configured_from"
)]
pub struct ConfiguredFrom {
    /// Deployed instance context ref.
    pub deployed_context_ref: ContextRefValue,
    /// Deployed instance address.
    pub deployed_address: ContractAddress,
}

/// Optional configured-state evidence snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configuration-snapshot",
    schema = "mfm.evm.contract.value.configuration_snapshot"
)]
pub struct ConfigurationSnapshot {
    /// Read assertion results proving configured state.
    pub read_results: Vec<ValidationReadResult>,
    /// Event assertion results proving configured state.
    pub event_results: Vec<ValidationEventResult>,
}

/// Context-bound configured contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance",
    schema = "mfm.evm.contract.value.configured_contract_instance"
)]
pub struct ConfiguredContractInstance {
    /// Lifecycle value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Deployed instance identity that configuration builds from.
    pub configured_from: ConfiguredFrom,
    /// Honest configuration provenance claim.
    pub configuration_claim: ConfigurationClaim,
    /// Configuration or import evidence refs.
    pub configure_or_import_evidence: Vec<LifecycleArtifactEvidenceRef>,
    /// Replayable external-adoption evidence when this instance was adopted externally.
    pub external_adoption_evidence: Option<ExternalAdoptionEvidence>,
    /// Optional highest block number that confirmed configuration or adoption.
    pub configured_block_number: Option<u64>,
    /// Optional configured-state assertion snapshot.
    pub asserted_configuration_snapshot: Option<ConfigurationSnapshot>,
}

/// Configured instance identity consumed by validation reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance-ref",
    schema = "mfm.evm.contract.value.configured_contract_instance_ref"
)]
pub struct ConfiguredContractInstanceRef {
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized configured contract address.
    pub address: ContractAddress,
    /// Honest configuration provenance claim.
    pub configuration_claim: ConfigurationClaim,
}

impl ConfiguredContractInstanceRef {
    /// Builds a configured instance reference from a configured instance.
    pub fn from_configured(configured: &ConfiguredContractInstance) -> Self {
        Self {
            context_ref: configured.context_ref.clone(),
            address: configured.address.clone(),
            configuration_claim: configured.configuration_claim.clone(),
        }
    }
}

/// Context-bound terminal validation report for a configured contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-bound-validation-report",
    schema = "mfm.evm.contract.value.context_bound_validation_report"
)]
pub struct ContextBoundValidationReport {
    /// Validation report contract version.
    pub report_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Configured contract instance that was validated.
    pub configured_instance: ConfiguredContractInstanceRef,
    /// Observed EVM chain id from validation read evidence.
    pub observed_chain_id: u64,
    /// Results for configuration-intent read confirmations stored on the configured instance.
    pub configuration_read_results: Vec<ValidationReadResult>,
    /// Results for configuration-intent event confirmations stored on the configured instance.
    pub configuration_event_results: Vec<ValidationEventResult>,
    /// Additional read assertion results from validation action.
    pub read_results: Vec<ValidationReadResult>,
    /// Additional event assertion results from validation action.
    pub event_results: Vec<ValidationEventResult>,
    /// Replayable read assertion evidence from validation reads.
    pub validation_read_evidence: Vec<ExternalReadAssertionEvidence>,
    /// Replayable event assertion evidence from validation log reads.
    pub validation_event_evidence: Vec<ExternalEventAssertionEvidence>,
    /// Retained lifecycle evidence refs that support the configured input being validated.
    pub evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Whether all validation checks passed.
    pub valid: bool,
}
