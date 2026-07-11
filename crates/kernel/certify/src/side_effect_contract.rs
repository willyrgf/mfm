use super::*;

/// Certified side-effect resource and remediation contract for one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedSideEffectContract {
    node_id: NodeId,
    side_effect: spec::SideEffectContractSpec,
    remediation_forward_node_id: Option<NodeId>,
}

impl CertifiedSideEffectContract {
    /// Mints a side-effect contract from a certified typed spec and node id.
    pub fn for_node(spec: &spec::TypedExecutionSpec, node_id: &NodeId) -> Result<Self> {
        let node = spec
            .nodes
            .iter()
            .chain(spec.remediations.values())
            .find(|node| node.node_id == *node_id)
            .ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!("side-effect contract requested for uncertified node {node_id}"),
                )
            })?;
        let side_effect = node.side_effect.clone().ok_or_else(|| {
            problem(
                ProblemClass::InvalidSemanticTransition,
                format!("node {node_id} has no certified side-effect contract"),
            )
        })?;
        let remediation_forward_node_id =
            spec.remediations
                .iter()
                .find_map(|(forward_node_id, remediation)| {
                    (remediation.node_id == *node_id).then_some(forward_node_id.clone())
                });
        Ok(Self {
            node_id: node_id.clone(),
            side_effect,
            remediation_forward_node_id,
        })
    }

    /// Returns the node id this contract certifies.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the certified side-effect resource claim.
    pub fn resource_claim(&self) -> &spec::ResourceClaimSpec {
        &self.side_effect.resource_claim
    }

    /// Returns the certified terminal verification policy.
    pub fn verification(&self) -> &spec::SideEffectVerificationSpec {
        &self.side_effect.verification
    }

    /// Validates whether a ledger purpose is admissible for this node.
    pub fn validate_ledger_purpose(&self, purpose: &events::SideEffectLedgerPurpose) -> Result<()> {
        match (&self.remediation_forward_node_id, purpose) {
            (None, events::SideEffectLedgerPurpose::Forward)
            | (Some(_), events::SideEffectLedgerPurpose::Remediation { .. }) => Ok(()),
            (None, events::SideEffectLedgerPurpose::Remediation { .. }) => Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "forward side-effect node {} emitted remediation ledger purpose",
                    self.node_id
                ),
            )),
            (Some(_), events::SideEffectLedgerPurpose::Forward) => Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} emitted forward ledger purpose",
                    self.node_id
                ),
            )),
        }
    }

    /// Validates exclusive resource-key evidence against the certified resource claim.
    pub fn validate_resource_key(
        &self,
        resource_key: Option<&events::ResourceKeyEvidence>,
    ) -> Result<()> {
        match &self.side_effect.resource_claim {
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            } => {
                let Some(resource_key) = resource_key else {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exclusive side-effect node {} recorded without resource key evidence",
                            self.node_id
                        ),
                    ));
                };
                if &resource_key.namespace != namespace || &resource_key.key_schema_id != key_schema
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exclusive side-effect node {} recorded resource key evidence outside certified schema",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
            spec::ResourceClaimSpec::ExactTouchedSet { .. }
            | spec::ResourceClaimSpec::ManualOnly => {
                if resource_key.is_some() {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded exclusive resource key without an exclusive certified resource claim",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Validates exact touched-set evidence against the certified resource claim.
    pub fn validate_touched_set(
        &self,
        touched_set: Option<&events::ResourceTouchedSetEvidence>,
    ) -> Result<()> {
        match &self.side_effect.resource_claim {
            spec::ResourceClaimSpec::ExactTouchedSet {
                namespace,
                evidence_schema,
            } => {
                let Some(touched_set) = touched_set else {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "exact-touched-set side-effect node {} recorded without touched-set evidence",
                            self.node_id
                        ),
                    ));
                };
                if &touched_set.namespace != namespace
                    || &touched_set.evidence_schema_id != evidence_schema
                {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded touched-set evidence outside certified schema",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
            spec::ResourceClaimSpec::Exclusive { .. } | spec::ResourceClaimSpec::ManualOnly => {
                if touched_set.is_some() {
                    return Err(problem(
                        ProblemClass::InvalidSemanticTransition,
                        format!(
                            "side-effect node {} recorded touched-set evidence without an exact-touched-set certified resource claim",
                            self.node_id
                        ),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Validates that a resource key remains stable across invocation epochs.
    pub fn validate_epoch_resource_consistency(
        &self,
        previous: Option<&events::ResourceKeyEvidence>,
        current: Option<&events::ResourceKeyEvidence>,
    ) -> Result<()> {
        self.validate_resource_key(current)?;
        if let (Some(previous), Some(current)) = (previous, current) {
            if previous != current {
                return Err(problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "exclusive side-effect node {} changed resource key across invocation epochs",
                        self.node_id
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Validates a remediation ledger link against certified forward/remediation relations.
    pub fn validate_remediation_link(&self, link: CertifiedRemediationLink<'_>) -> Result<()> {
        self.validate_ledger_purpose(link.ledger_purpose)?;
        let events::SideEffectLedgerPurpose::Remediation { .. } = link.ledger_purpose else {
            return Ok(());
        };
        let expected_forward_node_id =
            self.remediation_forward_node_id.as_ref().ok_or_else(|| {
                problem(
                    ProblemClass::InvalidSemanticTransition,
                    format!(
                        "forward side-effect node {} cannot emit remediation ledger purpose",
                        self.node_id
                    ),
                )
            })?;
        let Some(forward_run_id) = link.forward_run_id else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward ledger",
                    self.node_id
                ),
            ));
        };
        let Some(forward_node_id) = link.forward_node_id else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward node",
                    self.node_id
                ),
            ));
        };
        let Some(forward_ledger_purpose) = link.forward_ledger_purpose else {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked missing forward ledger purpose",
                    self.node_id
                ),
            ));
        };
        if forward_run_id != link.remediation_run_id
            || forward_node_id != expected_forward_node_id
            || !matches!(
                forward_ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
            || !link.forward_terminal
        {
            return Err(problem(
                ProblemClass::InvalidSemanticTransition,
                format!(
                    "remediation node {} linked ledger outside certified terminal forward node {}",
                    self.node_id, expected_forward_node_id
                ),
            ));
        }
        Ok(())
    }
}

/// Projection facts needed to certify a remediation ledger link without depending on a store type.
#[derive(Debug, Clone, Copy)]
pub struct CertifiedRemediationLink<'a> {
    /// Run id of the remediation ledger.
    pub remediation_run_id: &'a RunId,
    /// Ledger purpose recorded by the remediation event or projection.
    pub ledger_purpose: &'a events::SideEffectLedgerPurpose,
    /// Run id of the linked forward ledger, when projected.
    pub forward_run_id: Option<&'a RunId>,
    /// Node id that owns the linked forward ledger, when projected.
    pub forward_node_id: Option<&'a NodeId>,
    /// Ledger purpose of the linked forward ledger, when projected.
    pub forward_ledger_purpose: Option<&'a events::SideEffectLedgerPurpose>,
    /// Whether the linked forward ledger has certified terminal side-effect evidence.
    pub forward_terminal: bool,
}
