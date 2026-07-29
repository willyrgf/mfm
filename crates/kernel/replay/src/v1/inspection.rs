use mfm_canonical::{CanonicalValue, RecoverabilityContractV2, ValidatedCanonicalValueV2};
use mfm_ids::{EffectKey, RunId, SchemaId, StableId};
use mfm_journal::v1::{
    AccessAuditStatus, AuthorizationRef, CapabilityBindingRef, JournalHead, ObservationRef,
    SafeFailure, ValueRef,
};

use super::Result;

const TRANSITION_TRACE_CONTRACT: &str = "mfm.transition-trace.v1";

/// One transition trace validated by the frozen recoverability annex.
///
/// Retained values are inlined in this value. It grants no follow-up object
/// access.
#[derive(Clone, PartialEq, Eq)]
pub struct CanonicalTransitionTrace {
    validated: ValidatedCanonicalValueV2,
}

impl CanonicalTransitionTrace {
    pub(crate) fn encode(value: &CanonicalValue) -> Result<Self> {
        let validated =
            RecoverabilityContractV2::embedded()?.encode(TRANSITION_TRACE_CONTRACT, value)?;
        Ok(Self { validated })
    }

    /// Strictly decodes exact canonical transition-trace bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = RecoverabilityContractV2::embedded()?
            .strict_decode(TRANSITION_TRACE_CONTRACT, bytes)?;
        Ok(Self { validated })
    }

    /// Returns exact annex-validated canonical bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the annex-derived trace schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Reconstructs the canonical trace tree.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated.canonical_value().map_err(Into::into)
    }
}

impl std::fmt::Debug for CanonicalTransitionTrace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalTransitionTrace")
            .field("schema_id", self.schema_id())
            .finish_non_exhaustive()
    }
}

/// One head-fixed transition-trace page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionTracePage {
    run_id: RunId,
    at_journal_head: JournalHead,
    transitions: Vec<CanonicalTransitionTrace>,
    has_more: bool,
    next_index: Option<u32>,
}

impl TransitionTracePage {
    pub(crate) fn new(
        run_id: RunId,
        at_journal_head: JournalHead,
        transitions: Vec<CanonicalTransitionTrace>,
        has_more: bool,
        next_index: Option<u32>,
    ) -> Self {
        Self {
            run_id,
            at_journal_head,
            transitions,
            has_more,
            next_index,
        }
    }

    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the physical head fixed by the first page.
    pub const fn at_journal_head(&self) -> &JournalHead {
        &self.at_journal_head
    }

    /// Returns transition traces in committed order.
    pub fn transitions(&self) -> &[CanonicalTransitionTrace] {
        &self.transitions
    }

    /// Returns whether another transition exists after this page.
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    /// Returns the next zero-based transition index when another page exists.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }

    /// Consumes the inspected page into its exact owned scalar and trace parts.
    pub fn into_parts(
        self,
    ) -> (
        RunId,
        JournalHead,
        Vec<CanonicalTransitionTrace>,
        bool,
        Option<u32>,
    ) {
        (
            self.run_id,
            self.at_journal_head,
            self.transitions,
            self.has_more,
            self.next_index,
        )
    }
}

/// One owned public audit projection derived from a store-verified run view.
///
/// This is deliberately distinct from the persisted journal audit entry. The
/// journal retains only the delivery-audit reference, while this projection
/// carries the verified pending/terminal annotation and the complete reviewed
/// authorization and observation identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuditEntry {
    authorization_ref: AuthorizationRef,
    observation_ref: Option<ObservationRef>,
    authorization_journal_head: JournalHead,
    observation_journal_head: Option<JournalHead>,
    capability_binding_ref: CapabilityBindingRef,
    capability_operation_id: StableId,
    request_ref: ValueRef,
    status: AccessAuditStatus,
    result_ref: Option<ValueRef>,
    failure: Option<SafeFailure>,
    effect_key: Option<EffectKey>,
    delivery_audit_ref: Option<ValueRef>,
    delivery_audit_terminal: Option<bool>,
}

impl AccessAuditEntry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        authorization_ref: AuthorizationRef,
        observation_ref: Option<ObservationRef>,
        authorization_journal_head: JournalHead,
        observation_journal_head: Option<JournalHead>,
        capability_binding_ref: CapabilityBindingRef,
        capability_operation_id: StableId,
        request_ref: ValueRef,
        status: AccessAuditStatus,
        result_ref: Option<ValueRef>,
        failure: Option<SafeFailure>,
        effect_key: Option<EffectKey>,
        delivery_audit_ref: Option<ValueRef>,
        delivery_audit_terminal: Option<bool>,
    ) -> Self {
        Self {
            authorization_ref,
            observation_ref,
            authorization_journal_head,
            observation_journal_head,
            capability_binding_ref,
            capability_operation_id,
            request_ref,
            status,
            result_ref,
            failure,
            effect_key,
            delivery_audit_ref,
            delivery_audit_terminal,
        }
    }

    /// Returns the exact authorization record reference.
    pub const fn authorization_ref(&self) -> &AuthorizationRef {
        &self.authorization_ref
    }

    /// Returns the exact observation record reference, when observed as of the page head.
    pub const fn observation_ref(&self) -> Option<&ObservationRef> {
        self.observation_ref.as_ref()
    }

    /// Returns the physical head containing the authorization.
    pub const fn authorization_journal_head(&self) -> &JournalHead {
        &self.authorization_journal_head
    }

    /// Returns the physical head containing the observation, when observed as of the page head.
    pub const fn observation_journal_head(&self) -> Option<&JournalHead> {
        self.observation_journal_head.as_ref()
    }

    /// Returns the immutable admitted capability binding.
    pub const fn capability_binding_ref(&self) -> &CapabilityBindingRef {
        &self.capability_binding_ref
    }

    /// Returns the exact reviewed capability operation.
    pub const fn capability_operation_id(&self) -> &StableId {
        &self.capability_operation_id
    }

    /// Returns the full retained request authority.
    pub const fn request_ref(&self) -> &ValueRef {
        &self.request_ref
    }

    /// Returns the closed safe audit status.
    pub const fn status(&self) -> AccessAuditStatus {
        self.status
    }

    /// Returns the full retained result authority for a reviewed return.
    pub const fn result_ref(&self) -> Option<&ValueRef> {
        self.result_ref.as_ref()
    }

    /// Returns the reviewed bounded failure, when present.
    pub const fn failure(&self) -> Option<&SafeFailure> {
        self.failure.as_ref()
    }

    /// Returns the immutable effect key for executor access, when present.
    pub const fn effect_key(&self) -> Option<&EffectKey> {
        self.effect_key.as_ref()
    }

    /// Returns the greatest verified executor delivery-audit head, when present.
    pub const fn delivery_audit_ref(&self) -> Option<&ValueRef> {
        self.delivery_audit_ref.as_ref()
    }

    /// Returns whether the verified returned delivery audit is terminal.
    ///
    /// `Some(true)` identifies terminal evidence, `Some(false)` identifies a
    /// pending ensure result, and `None` identifies a read or an authorization
    /// without a verified returned ensure result as of the page head.
    pub const fn delivery_audit_terminal(&self) -> Option<bool> {
        self.delivery_audit_terminal
    }
}

/// One head-fixed safe external-access audit page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuditPage {
    run_id: RunId,
    complete_as_of_journal_head: JournalHead,
    entries: Vec<AccessAuditEntry>,
    has_more: bool,
    next_index: Option<u32>,
}

impl AccessAuditPage {
    pub(crate) fn new(
        run_id: RunId,
        complete_as_of_journal_head: JournalHead,
        entries: Vec<AccessAuditEntry>,
        has_more: bool,
        next_index: Option<u32>,
    ) -> Self {
        Self {
            run_id,
            complete_as_of_journal_head,
            entries,
            has_more,
            next_index,
        }
    }

    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the complete physical head fixed by the first page.
    pub const fn complete_as_of_journal_head(&self) -> &JournalHead {
        &self.complete_as_of_journal_head
    }

    /// Returns safe entries in authorization order.
    pub fn entries(&self) -> &[AccessAuditEntry] {
        &self.entries
    }

    /// Returns whether another entry exists after this page.
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    /// Returns the next zero-based authorization index when another page exists.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }

    /// Consumes the inspected page into its exact owned scalar and entry parts.
    pub fn into_parts(self) -> (RunId, JournalHead, Vec<AccessAuditEntry>, bool, Option<u32>) {
        (
            self.run_id,
            self.complete_as_of_journal_head,
            self.entries,
            self.has_more,
            self.next_index,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use mfm_ids::{
        ArtifactId, ContentDigest, ContentRef, EffectKey, JournalCommitDigest, JournalRecordHash,
        ObjectEvidenceDigest, RunId, SchemaId, SemanticTypeId, StableId,
    };
    use mfm_journal::v1::{
        AccessAuditStatus, AuthorizationRef, CapabilityBindingRef, JournalHead, ObservationRef,
        ProducerBinding, RecordRef, ValueRef,
    };

    use super::{AccessAuditEntry, TransitionTracePage};

    #[test]
    fn trace_page_exposes_only_fixed_head_and_numeric_continuation() {
        let run_id = run_id('0');
        let head = JournalHead::new(4, &commit_digest('1')).expect("head");
        let page =
            TransitionTracePage::new(run_id.clone(), head.clone(), Vec::new(), true, Some(12));

        assert_eq!(page.run_id(), &run_id);
        assert_eq!(page.at_journal_head(), &head);
        assert!(page.transitions().is_empty());
        assert!(page.has_more());
        assert_eq!(page.next_index(), Some(12));

        let (actual_run_id, actual_head, traces, has_more, next_index) = page.into_parts();
        assert_eq!(actual_run_id, run_id);
        assert_eq!(actual_head, head);
        assert!(traces.is_empty());
        assert!(has_more);
        assert_eq!(next_index, Some(12));
    }

    #[test]
    fn public_audit_entry_owns_every_reviewed_field_and_terminal_annotation() {
        let run_id = run_id('1');
        let authorization_ref =
            AuthorizationRef::new(&record_ref(&run_id, 2, '2')).expect("authorization ref");
        let observation_ref =
            ObservationRef::new(&record_ref(&run_id, 3, '3')).expect("observation ref");
        let authorization_head = JournalHead::new(2, &commit_digest('4')).expect("head");
        let observation_head = JournalHead::new(3, &commit_digest('5')).expect("head");
        let capability_binding_ref =
            CapabilityBindingRef::new(&content_ref('6')).expect("capability binding");
        let capability_operation_id =
            StableId::new("mfm.test/audit-operation").expect("operation id");
        let request_ref = value_ref('7');
        let result_ref = value_ref('8');
        let effect_key =
            EffectKey::from_str(&format!("effect:sha256-jcs-v1:{}", repeated_hex('9')))
                .expect("effect key");
        let delivery_audit_ref = value_ref('a');

        let terminal = AccessAuditEntry::new(
            authorization_ref.clone(),
            Some(observation_ref.clone()),
            authorization_head.clone(),
            Some(observation_head.clone()),
            capability_binding_ref.clone(),
            capability_operation_id.clone(),
            request_ref.clone(),
            AccessAuditStatus::Returned,
            Some(result_ref.clone()),
            None,
            Some(effect_key.clone()),
            Some(delivery_audit_ref.clone()),
            Some(true),
        );

        assert_eq!(terminal.authorization_ref(), &authorization_ref);
        assert_eq!(terminal.observation_ref(), Some(&observation_ref));
        assert_eq!(terminal.authorization_journal_head(), &authorization_head);
        assert_eq!(terminal.observation_journal_head(), Some(&observation_head));
        assert_eq!(terminal.capability_binding_ref(), &capability_binding_ref);
        assert_eq!(terminal.capability_operation_id(), &capability_operation_id);
        assert_eq!(terminal.request_ref(), &request_ref);
        assert_eq!(terminal.status(), AccessAuditStatus::Returned);
        assert_eq!(terminal.result_ref(), Some(&result_ref));
        assert_eq!(terminal.failure(), None);
        assert_eq!(terminal.effect_key(), Some(&effect_key));
        assert_eq!(terminal.delivery_audit_ref(), Some(&delivery_audit_ref));
        assert_eq!(terminal.delivery_audit_terminal(), Some(true));

        let pending = AccessAuditEntry::new(
            authorization_ref.clone(),
            Some(observation_ref),
            authorization_head.clone(),
            Some(observation_head),
            capability_binding_ref.clone(),
            capability_operation_id.clone(),
            request_ref.clone(),
            AccessAuditStatus::Returned,
            Some(result_ref),
            None,
            Some(effect_key.clone()),
            Some(delivery_audit_ref),
            Some(false),
        );
        assert_eq!(pending.delivery_audit_terminal(), Some(false));

        let unobserved = AccessAuditEntry::new(
            authorization_ref,
            None,
            authorization_head,
            None,
            capability_binding_ref,
            capability_operation_id,
            request_ref,
            AccessAuditStatus::AuthorizedUnobserved,
            None,
            None,
            Some(effect_key),
            None,
            None,
        );
        assert_eq!(unobserved.observation_ref(), None);
        assert_eq!(unobserved.observation_journal_head(), None);
        assert_eq!(unobserved.result_ref(), None);
        assert_eq!(unobserved.delivery_audit_ref(), None);
        assert_eq!(unobserved.delivery_audit_terminal(), None);
    }

    fn run_id(hex: char) -> RunId {
        RunId::from_str(&format!("run:sha256-jcs-v1:{}", repeated_hex(hex))).expect("run id")
    }

    fn commit_digest(hex: char) -> JournalCommitDigest {
        JournalCommitDigest::from_str(&format!("sha256-jcs-v1:{}", repeated_hex(hex)))
            .expect("commit digest")
    }

    fn record_ref(run_id: &RunId, run_sequence: u64, character: char) -> RecordRef {
        let hash =
            JournalRecordHash::from_str(&format!("sha256-jcs-v1:{}", repeated_hex(character)))
                .expect("record hash");
        RecordRef::new(run_id, run_sequence, 0, &hash).expect("record ref")
    }

    fn content_ref(character: char) -> ContentRef {
        let schema_id = SchemaId::from_str(&format!(
            "schema:mfm.test-audit-value:1:sha256-jcs-v1:{}",
            repeated_hex(character)
        ))
        .expect("schema id");
        let digest =
            ContentDigest::from_str(&format!("content:sha256-v1:{}", repeated_hex(character)))
                .expect("content digest");
        ContentRef::new(schema_id, digest).expect("content ref")
    }

    fn value_ref(character: char) -> ValueRef {
        let artifact_id = ArtifactId::from_str(&format!(
            "artifact:sha256-jcs-v1:{}",
            repeated_hex(character)
        ))
        .expect("artifact id");
        let content = content_ref(character);
        let evidence_hash =
            ObjectEvidenceDigest::from_str(&format!("sha256-jcs-v1:{}", repeated_hex(character)))
                .expect("evidence hash");
        let semantic_type_id = SemanticTypeId::from_str(&format!(
            "semantic:mfm:test-audit-value:1:sha256-jcs-v1:{}",
            repeated_hex(character)
        ))
        .expect("semantic type id");
        let role = StableId::new("audit-test").expect("role");
        let producer =
            ProducerBinding::this_admission(&StableId::new("audit-test-slot").expect("slot"))
                .expect("producer binding");
        ValueRef::new(
            &artifact_id,
            content.content_digest(),
            &evidence_hash,
            content.schema_id(),
            &semantic_type_id,
            &role,
            1,
            "application/json",
            &content_ref('b'),
            &producer,
        )
        .expect("value ref")
    }

    fn repeated_hex(character: char) -> String {
        character.to_string().repeat(64)
    }
}
