use super::*;

/// Context-bound state that imports a deployed contract instance from certified evidence.
pub struct ImportDeployedContractState;

impl ImportDeployedContractState {
    /// Projects a deployed instance from already verified source-run evidence.
    pub fn admit_verified_mfm_run_import(
        import: &ImportDeployedSpec,
        imported: DeployedContractInstance,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<DeployedContractInstance> {
        let ImportDeployedSpec::FromMfmRun {
            source: _,
            evidence,
        } = import
        else {
            return Err(import_admission_error(Self::name()));
        };
        ensure_verified_source_run_import(evidence, &imported, ContractLifecycleStage::Deployed)?;
        Ok(DeployedContractInstance {
            lifecycle_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            address: imported.address,
            deploy_provenance: DeployProvenance::ImportedMfmRun {
                source_context_ref: evidence.source_context_ref.clone(),
                source_value_digest: evidence.source_value_digest.clone(),
                import_policy_digest: evidence.import_policy_digest.clone(),
            },
            deploy_evidence: source_run_import_evidence_refs(evidence),
            external_adoption_evidence: None,
            deployed_block_number: imported.deployed_block_number,
        })
    }

    /// Projects a deployed instance from already verified external adoption evidence.
    pub fn admit_verified_external_adoption(
        import: &ImportDeployedSpec,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        evidence: ExternalAdoptionEvidence,
        deployed_block_number: Option<u64>,
    ) -> StateResult<DeployedContractInstance> {
        let ImportDeployedSpec::AdoptExternalAddress { adoption } = import else {
            return Err(import_admission_error(Self::name()));
        };
        deployed_instance_from_verified_external_adoption(
            adoption,
            context,
            evidence,
            deployed_block_number,
        )
    }
}

impl StateSpec for ImportDeployedContractState {
    type Config = ImportDeployedSpec;
    type Context = EvmContractContext;
    type Input = ();
    type Output = DeployedContractInstance;
    type Effect = ReadExternal;
    type Caps = ContractImportReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("import_deployed")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("import_deployed")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.import_deployed"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        produces_context(contract_instance_resource_kind(), deployed_contract_stage())
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ReadState for ImportDeployedContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(import_admission_error(Self::name())))
    }
}

/// Context-bound state that imports a configured contract instance from certified evidence.
pub struct ImportConfiguredContractState;

impl ImportConfiguredContractState {
    /// Projects a configured instance from already verified source-run evidence.
    pub fn admit_verified_mfm_run_import(
        import: &ImportConfiguredSpec,
        imported: ConfiguredContractInstance,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<ConfiguredContractInstance> {
        let ImportConfiguredSpec::FromMfmRun { source, evidence } = import else {
            return Err(import_admission_error(Self::name()));
        };
        ensure_verified_source_run_import(evidence, &imported, ContractLifecycleStage::Configured)?;
        Ok(ConfiguredContractInstance {
            lifecycle_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            address: imported.address.clone(),
            configured_from: ConfiguredFrom {
                deployed_context_ref: ContextRefValue::from(context.context_ref().clone()),
                deployed_address: imported.address,
            },
            configuration_claim: ConfigurationClaim::ImportedMfmConfigured {
                source_run_id: source.source_run_id.clone(),
                source_spec_hash: evidence.source_spec_hash.clone(),
                source_cell_or_output_id: evidence.source_cell_or_output_id.clone(),
                source_value_digest: evidence.source_value_digest.clone(),
                source_context_ref: evidence.source_context_ref.clone(),
            },
            configure_or_import_evidence: source_run_import_evidence_refs(evidence),
            external_adoption_evidence: None,
            configured_block_number: imported.configured_block_number,
            asserted_configuration_snapshot: None,
        })
    }

    /// Projects a configured instance from already verified external adoption evidence.
    pub fn admit_verified_external_adoption(
        import: &ImportConfiguredSpec,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        evidence: ExternalAdoptionEvidence,
        snapshot: Option<ConfigurationSnapshot>,
        configured_block_number: Option<u64>,
    ) -> StateResult<ConfiguredContractInstance> {
        let ImportConfiguredSpec::AdoptExternalAddress { adoption } = import else {
            return Err(import_admission_error(Self::name()));
        };
        ensure_external_adoption_evidence(
            &evidence,
            context,
            ContractLifecycleStage::Configured,
            adoption,
        )?;
        let evidence_snapshot = ConfigurationSnapshot {
            read_results: evidence
                .read_assertion_evidence
                .iter()
                .map(|record| record.result.clone())
                .collect(),
            event_results: evidence
                .event_assertion_evidence
                .iter()
                .map(|record| record.result.clone())
                .collect(),
        };
        if let Some(snapshot) = snapshot.as_ref() {
            if snapshot != &evidence_snapshot {
                return Err(import_admission_error(Self::name()));
            }
        } else if !evidence_snapshot.read_results.is_empty()
            || !evidence_snapshot.event_results.is_empty()
        {
            return Err(import_admission_error(Self::name()));
        }
        let assertions_required = !adoption.evidence_policy.initial_read_assertions.is_empty()
            || !adoption.evidence_policy.initial_event_assertions.is_empty();
        let (configuration_claim, asserted_configuration_snapshot) = if assertions_required {
            let Some(snapshot) = snapshot else {
                return Err(import_admission_error(Self::name()));
            };
            (
                ConfigurationClaim::ExternalObservedConfigured {
                    provenance_label: adoption.provenance_label.clone(),
                    evidence_policy_digest: evidence.evidence_policy_digest.clone(),
                    external_adoption_evidence_digest: digest_for_config(&evidence)?,
                },
                Some(snapshot),
            )
        } else if let Some(snapshot) = snapshot {
            (
                ConfigurationClaim::ExternalObservedConfigured {
                    provenance_label: adoption.provenance_label.clone(),
                    evidence_policy_digest: evidence.evidence_policy_digest.clone(),
                    external_adoption_evidence_digest: digest_for_config(&evidence)?,
                },
                Some(snapshot),
            )
        } else if adoption.evidence_policy.allow_external_claimed_configured {
            (
                ConfigurationClaim::ExternalClaimedConfigured {
                    provenance_label: adoption.provenance_label.clone(),
                    evidence_policy_digest: evidence.evidence_policy_digest.clone(),
                },
                None,
            )
        } else {
            return Err(import_admission_error(Self::name()));
        };

        Ok(ConfiguredContractInstance {
            lifecycle_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            address: adoption.address.clone(),
            configured_from: ConfiguredFrom {
                deployed_context_ref: ContextRefValue::from(context.context_ref().clone()),
                deployed_address: adoption.address.clone(),
            },
            configuration_claim,
            configure_or_import_evidence: Vec::new(),
            external_adoption_evidence: Some(evidence),
            configured_block_number,
            asserted_configuration_snapshot,
        })
    }
}

impl StateSpec for ImportConfiguredContractState {
    type Config = ImportConfiguredSpec;
    type Context = EvmContractContext;
    type Input = ();
    type Output = ConfiguredContractInstance;
    type Effect = ReadExternal;
    type Caps = ContractImportReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("import_configured")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("import_configured")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.import_configured"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        produces_context(
            contract_instance_resource_kind(),
            configured_contract_stage(),
        )
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ReadState for ImportConfiguredContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(import_admission_error(Self::name())))
    }
}
