//! Sealed purpose-specific run history readers and evidence.
//!
//! Each purpose reader loads through the same callback-free fold, then wraps the
//! verified prefix in a purpose-sealed evidence newtype. Evidence types have no
//! public field, no `Deref`, and expose only the accessors that purpose needs.
//! Cross-purpose substitution is a type error: `PublicRunEvidence` cannot be
//! passed where `ExportRunEvidence` is required.

use mfm_ids::{ContentRef, RunId};
use mfm_journal::structured::{
    AssignedRecord, HistoryObject, JournalHead, RunAdmitted, SemanticHead,
};

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryReader};
use super::fold::{StructuredFrontier, VerifiedStructuredRun};
use super::Result;

macro_rules! purpose_reader_shell {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        pub struct $name<B: StructuredHistoryBackend> {
            reader: StructuredRunHistoryReader<B>,
        }

        impl<B: StructuredHistoryBackend> $name<B> {
            pub(super) fn new(reader: StructuredRunHistoryReader<B>) -> Self {
                Self { reader }
            }

            /// Returns the immutable qualified store identity.
            pub fn store_identity(&self) -> &super::StructuredStoreIdentity {
                self.reader.store_identity()
            }

            /// Probes backend readability without requiring an existing application run.
            pub async fn check_ready(&self) -> Result<()> {
                self.reader.check_ready().await
            }
        }
    };
}

purpose_reader_shell!(
    /// Target-bound public-read projection authority.
    PublicRunReader
);
purpose_reader_shell!(
    /// Target-bound transition-trace projection authority.
    TraceRunReader
);
purpose_reader_shell!(
    /// Target-bound access-audit projection authority.
    AuditRunReader
);
purpose_reader_shell!(
    /// Target-bound recorded-replay projection authority.
    ReplayRunReader
);
purpose_reader_shell!(
    /// Target-bound export projection authority.
    ExportRunReader
);

impl<B: StructuredHistoryBackend> PublicRunReader<B> {
    /// Loads one verified run as public-read evidence only.
    pub async fn load_public(&self, run_id: &RunId) -> Result<PublicRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(PublicRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> TraceRunReader<B> {
    /// Loads one verified run as transition-trace evidence only.
    pub async fn load_transition_trace(&self, run_id: &RunId) -> Result<TraceRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(TraceRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> AuditRunReader<B> {
    /// Loads one verified run as access-audit evidence only.
    pub async fn load_access_audit(&self, run_id: &RunId) -> Result<AuditRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(AuditRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> ReplayRunReader<B> {
    /// Loads one verified run as recorded-replay evidence only.
    pub async fn load_for_recorded_verify(&self, run_id: &RunId) -> Result<RecordedRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(RecordedRunEvidence::from_verified)
    }
}

impl<B: StructuredHistoryBackend> ExportRunReader<B> {
    /// Loads one verified run as portable-export evidence only.
    pub async fn load_for_export(&self, run_id: &RunId) -> Result<ExportRunEvidence> {
        self.reader
            .load_verified(run_id)
            .await
            .map(ExportRunEvidence::from_verified)
    }
}

/// Sealed public-read evidence. Cannot be used as export, trace, audit, or replay evidence.
#[derive(Debug)]
pub struct PublicRunEvidence(VerifiedStructuredRun);

impl PublicRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.0.closed_outcome_ref()
    }

    /// Resolves one exact verified content-addressed history object.
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.0.object(content_ref)
    }
}

/// Sealed transition-trace evidence. Cannot be used as public, export, audit, or replay evidence.
#[derive(Debug)]
pub struct TraceRunEvidence(VerifiedStructuredRun);

impl TraceRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }
}

/// Sealed access-audit evidence. Cannot be used as public, export, trace, or replay evidence.
#[derive(Debug)]
pub struct AuditRunEvidence(VerifiedStructuredRun);

impl AuditRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }
}

/// Sealed recorded-replay evidence. Cannot be used as public, export, trace, or audit evidence.
#[derive(Debug)]
pub struct RecordedRunEvidence(VerifiedStructuredRun);

impl RecordedRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }
}

/// Sealed portable-export evidence. Cannot be used as public, trace, audit, or replay evidence.
#[derive(Debug)]
pub struct ExportRunEvidence(VerifiedStructuredRun);

impl ExportRunEvidence {
    fn from_verified(verified: VerifiedStructuredRun) -> Self {
        Self(verified)
    }

    /// Returns the exact run identity.
    pub const fn run_id(&self) -> &RunId {
        self.0.run_id()
    }

    /// Returns the exact verified admission root.
    pub const fn admission(&self) -> &RunAdmitted {
        self.0.admission()
    }

    /// Returns the exact physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.0.journal_head()
    }

    /// Returns the exact semantic head.
    pub const fn semantic_head(&self) -> &SemanticHead {
        self.0.semantic_head()
    }

    /// Returns the closed action frontier.
    pub const fn frontier(&self) -> &StructuredFrontier {
        self.0.frontier()
    }

    /// Returns every verified physical append head in sequence order.
    pub fn journal_heads(&self) -> &[JournalHead] {
        self.0.journal_heads()
    }

    /// Returns every verified assigned record in physical append order.
    pub fn records(&self) -> &[AssignedRecord] {
        self.0.records()
    }

    /// Returns the verified record prefix through the exact semantic head record.
    pub fn semantic_records(&self) -> impl Iterator<Item = &AssignedRecord> {
        self.0.semantic_records()
    }

    /// Returns verified objects first admitted no later than one physical append sequence.
    pub fn objects_through(&self, run_sequence: u64) -> impl Iterator<Item = &HistoryObject> {
        self.0.objects_through(run_sequence)
    }

    /// Resolves one exact verified content-addressed history object.
    pub fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject> {
        self.0.object(content_ref)
    }

    /// Returns the terminal nominal operation-outcome reference, when closed.
    pub const fn closed_outcome_ref(&self) -> Option<&ContentRef> {
        self.0.closed_outcome_ref()
    }

    /// Returns the sole callback-free cursor.
    pub fn cursor(&self) -> &super::fold::ProgramCursor {
        self.0.cursor()
    }
}
