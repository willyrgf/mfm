use super::super::*;
use super::CommittedJournalBatch;

/// Private physical fold over the current persisted journal format.
#[derive(Debug)]
pub(crate) struct JournalFold {
    batches: Vec<CommittedJournalBatch>,
    current_run_sequence: StreamSeq,
    admitted_spec_hash: SpecHash,
}

impl JournalFold {
    pub(super) fn rebuild(records: &[KernelEventEnvelope]) -> Result<Self> {
        ProjectionSnapshot::validate_run_stream(records)?;
        let admission = super::run_admission_root(records)?;
        let batches = super::committed_journal_batches(records);
        let current_run_sequence = records
            .last()
            .ok_or_else(|| StoreError::PersistedEventMismatch {
                field: "current_run_sequence",
                message: "committed journal has no current sequence".to_owned(),
            })?
            .seq();
        Ok(Self {
            batches,
            current_run_sequence,
            admitted_spec_hash: admission.spec_hash.clone(),
        })
    }

    pub(super) fn batches(&self) -> &[CommittedJournalBatch] {
        &self.batches
    }

    pub(super) fn current_run_sequence(&self) -> StreamSeq {
        self.current_run_sequence
    }

    pub(super) fn spec_hash(&self) -> &SpecHash {
        &self.admitted_spec_hash
    }
}
