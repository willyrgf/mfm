use super::*;
use std::collections::BTreeSet;

/// Finalized run-level saga policy recorded on a typed program draft.
///
/// Program builders derive [`SagaPolicy::NoSideEffects`] when the graph has no side-effecting
/// forward nodes. Authors select only [`SideEffectSagaPolicy`] for side-effecting workflows.
/// Certification lowers the finalized policy into the hash-defining `mfm-spec` contract and
/// re-verifies the hostile-bytes shape. Runtime decisions remain derived from this policy plus
/// recorded facts, not from author-emitted control events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaPolicy {
    /// Derived by finalization when the forward graph has no side-effect nodes.
    NoSideEffects,
    /// Failure after mutation carries no compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
    /// Failure after mutation blocks for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: ManualResolutionPolicyDraft,
    },
    /// Failure after confirmed forward side effects compensates linked remediations.
    CompensateCompleted {
        /// Directive used when a forward or remediation ledger remains unresolved.
        on_remediation_unresolved: RemediationUnresolved,
    },
}

/// Author-selectable saga policy for side-effecting workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffectSagaPolicy {
    /// Failure after mutation carries no compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
    /// Failure after mutation blocks for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: ManualResolutionPolicyDraft,
    },
    /// Failure after confirmed forward side effects compensates linked remediations.
    CompensateCompleted {
        /// Directive used when a forward or remediation ledger remains unresolved.
        on_remediation_unresolved: RemediationUnresolved,
    },
}

impl SideEffectSagaPolicy {
    pub(crate) fn is_compensating(&self) -> bool {
        matches!(self, Self::CompensateCompleted { .. })
    }
}

impl From<SideEffectSagaPolicy> for SagaPolicy {
    fn from(policy: SideEffectSagaPolicy) -> Self {
        match policy {
            SideEffectSagaPolicy::FailWithoutAcdcClaim => Self::FailWithoutAcdcClaim,
            SideEffectSagaPolicy::ManualResolution { manual } => Self::ManualResolution { manual },
            SideEffectSagaPolicy::CompensateCompleted {
                on_remediation_unresolved,
            } => Self::CompensateCompleted {
                on_remediation_unresolved,
            },
        }
    }
}

/// Directive for unresolved remediation under compensating policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemediationUnresolved {
    /// Block for typed operator evidence.
    ManualResolution {
        /// Required typed manual evidence.
        manual: Box<ManualResolutionPolicyDraft>,
    },
    /// Terminally fail without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

/// Typed schema requirements for run-scoped manual saga resolution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionPolicyDraft {
    evidence_schema: SchemaId,
    authorization: ManualAuthorizationDraft,
}

impl ManualResolutionPolicyDraft {
    /// Creates a manual-resolution policy draft from typed evidence and authorization authority.
    pub fn new(evidence_schema: SchemaId, authorization: ManualAuthorizationDraft) -> Self {
        Self {
            evidence_schema,
            authorization,
        }
    }

    /// Returns the schema id required for the operator evidence artifact.
    pub fn evidence_schema(&self) -> &SchemaId {
        &self.evidence_schema
    }

    /// Returns the authorization policy draft required for the manual decision.
    pub fn authorization(&self) -> &ManualAuthorizationDraft {
        &self.authorization
    }

    /// Returns the lowered manual-resolution evidence spec.
    pub fn to_spec(&self) -> ManualResolutionEvidenceSpec {
        ManualResolutionEvidenceSpec {
            evidence_schema: self.evidence_schema.clone(),
            authorization: self.authorization.to_spec(),
        }
    }
}

/// Certified authorization policy draft for run-scoped manual saga resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAuthorizationDraft {
    verifier_id: ManualAuthorizationVerifierId,
    signing_scheme: ManualSigningSchemeSpec,
    authority: OperatorAuthoritySnapshotDraft,
    quorum: ThresholdQuorum,
}

impl ManualAuthorizationDraft {
    /// Creates a threshold manual authorization draft.
    pub fn threshold(
        verifier_id: ManualAuthorizationVerifierId,
        signing_scheme: ManualSigningSchemeSpec,
        authority: OperatorAuthoritySnapshotDraft,
        quorum: ThresholdQuorum,
    ) -> Result<Self> {
        if quorum.required_signatures() as usize > authority.operator_count() {
            return Err(PlanError::ManualPolicy(format!(
                "manual authorization quorum {} exceeds operator authority size {}",
                quorum.required_signatures(),
                authority.operator_count()
            )));
        }
        Ok(Self {
            verifier_id,
            signing_scheme,
            authority,
            quorum,
        })
    }

    /// Returns the verifier identity.
    pub fn verifier_id(&self) -> &ManualAuthorizationVerifierId {
        &self.verifier_id
    }

    /// Returns the signing scheme.
    pub fn signing_scheme(&self) -> &ManualSigningSchemeSpec {
        &self.signing_scheme
    }

    /// Returns the operator authority snapshot draft.
    pub fn authority(&self) -> &OperatorAuthoritySnapshotDraft {
        &self.authority
    }

    /// Returns the threshold quorum.
    pub fn quorum(&self) -> ThresholdQuorum {
        self.quorum
    }

    fn to_spec(&self) -> ManualResolutionAuthorizationSpec {
        ManualResolutionAuthorizationSpec {
            verifier_id: self.verifier_id.clone(),
            signing_scheme: self.signing_scheme.clone(),
            authority: self.authority.to_spec(),
            quorum: ManualAuthorizationQuorumSpec::new(self.quorum.required_signatures())
                .expect("ThresholdQuorum is non-zero"),
        }
    }
}

/// Non-empty, operator-id-unique collection for manual authorization snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyUniqueOperators {
    operators: Vec<OperatorAuthorityMemberSpec>,
}

impl NonEmptyUniqueOperators {
    /// Creates a non-empty unique collection from a first operator and optional rest.
    pub fn new(
        first: OperatorAuthorityMemberSpec,
        mut rest: Vec<OperatorAuthorityMemberSpec>,
    ) -> Result<Self> {
        let mut operators = Vec::with_capacity(rest.len() + 1);
        operators.push(first);
        operators.append(&mut rest);
        Self::try_from_vec(operators)
    }

    /// Attempts to create a non-empty unique operator collection from a vector.
    pub fn try_from_vec(operators: Vec<OperatorAuthorityMemberSpec>) -> Result<Self> {
        if operators.is_empty() {
            return Err(PlanError::ManualPolicy(
                "manual operator authority must contain at least one operator".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for operator in &operators {
            if !seen.insert(operator.operator_id.as_str().to_owned()) {
                return Err(PlanError::ManualPolicy(format!(
                    "duplicate manual operator id {}",
                    operator.operator_id
                )));
            }
        }
        Ok(Self { operators })
    }

    /// Returns the operators in retained snapshot order.
    pub fn operators(&self) -> &[OperatorAuthorityMemberSpec] {
        &self.operators
    }

    fn into_vec(self) -> Vec<OperatorAuthorityMemberSpec> {
        self.operators
    }
}

/// Manual authorization threshold that requires at least one signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThresholdQuorum {
    required_signatures: u32,
}

impl ThresholdQuorum {
    /// Creates a non-zero threshold quorum.
    pub fn new(required_signatures: u32) -> Result<Self> {
        if required_signatures == 0 {
            return Err(PlanError::ManualPolicy(
                "manual authorization quorum must require at least one signature".to_owned(),
            ));
        }
        Ok(Self {
            required_signatures,
        })
    }

    /// Returns the required signature count.
    pub fn required_signatures(self) -> u32 {
        self.required_signatures
    }
}

/// Operator authority snapshot draft with a non-empty unique operator set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorAuthoritySnapshotDraft {
    authority_id: OperatorAuthorityId,
    operators: NonEmptyUniqueOperators,
}

impl OperatorAuthoritySnapshotDraft {
    /// Creates an operator authority snapshot draft.
    pub fn new(authority_id: OperatorAuthorityId, operators: NonEmptyUniqueOperators) -> Self {
        Self {
            authority_id,
            operators,
        }
    }

    /// Returns the authority id.
    pub fn authority_id(&self) -> &OperatorAuthorityId {
        &self.authority_id
    }

    /// Returns operators in retained snapshot order.
    pub fn operators(&self) -> &[OperatorAuthorityMemberSpec] {
        self.operators.operators()
    }

    fn operator_count(&self) -> usize {
        self.operators.operators().len()
    }

    fn to_spec(&self) -> OperatorAuthoritySnapshotSpec {
        OperatorAuthoritySnapshotSpec {
            authority_id: self.authority_id.clone(),
            operators: self.operators.clone().into_vec(),
        }
    }
}
