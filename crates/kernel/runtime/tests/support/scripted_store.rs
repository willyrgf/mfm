use mfm_ids::RunId;
use mfm_journal::{decode_frame, EncodedRunFrame};
use mfm_store::LoadedRun;
use mfm_store::{AppendResult, MemoryStore, RunSummary, Store, StoreError};
use std::{collections::VecDeque, future::Future, pin::Pin, sync::Arc};

#[derive(Clone, Copy)]
pub(super) enum AppendAction {
    RetainThenNotInserted,
    Indeterminate,
    RetainThenIndeterminate,
}

pub(super) struct ScriptedStore {
    inner: MemoryStore,
    actions: std::sync::Mutex<VecDeque<(u64, AppendAction)>>,
    frames: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl ScriptedStore {
    pub(super) fn new(actions: impl IntoIterator<Item = (u64, AppendAction)>) -> Self {
        Self {
            inner: MemoryStore::new(),
            actions: std::sync::Mutex::new(actions.into_iter().collect()),
            frames: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(super) fn recording() -> Self {
        Self::new([])
    }

    pub(super) fn snapshot(&self) -> Vec<Vec<u8>> {
        self.frames.lock().expect("recorded frames").clone()
    }

    fn record_if_inserted(&self, frame: &EncodedRunFrame, result: AppendResult) {
        if result == AppendResult::Inserted {
            self.frames
                .lock()
                .expect("recorded frames")
                .push(frame.canonical_bytes().to_vec());
        }
    }
}

impl Store for ScriptedStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Option<LoadedRun>, StoreError>> + Send + 'a>>
    {
        self.inner.load_run(run_id, probe_sequence)
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let action = {
                let mut actions = self.actions.lock().expect("append actions");
                actions
                    .iter()
                    .position(|(sequence, _)| *sequence == frame.run_sequence())
                    .and_then(|index| actions.remove(index))
                    .map(|(_, action)| action)
            };
            match action {
                Some(AppendAction::RetainThenNotInserted) => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Ok(AppendResult::NotInserted)
                }
                Some(AppendAction::Indeterminate) => Err(StoreError::Indeterminate(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
                    ),
                )),
                Some(AppendAction::RetainThenIndeterminate) => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Err(StoreError::Indeterminate(
                        mfm_values::DiagnosticEvidence::from_value(
                            serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
                        ),
                    ))
                }
                None => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Ok(result)
                }
            }
        })
    }
}

pub(super) struct RetainedStore(pub(super) Vec<Vec<u8>>);

impl Store for RetainedStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<LoadedRun>, StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(latest) = self.0.last() else {
                return Ok(None);
            };
            let frame = decode_frame(latest).map_err(|_| {
                StoreError::CorruptPhysicalState(mfm_values::DiagnosticEvidence::from_value(
                    serde_json::json!({"operation": "test.store", "injected": "CorruptPhysicalState"}),
                ))
            })?;
            let head = RunSummary::new(
                run_id.clone(),
                frame.run_sequence(),
                frame.head_digest().clone(),
                self.0.iter().map(|bytes| bytes.len() as u64).sum(),
            )
            .map_err(|_| {
                StoreError::CorruptPhysicalState(mfm_values::DiagnosticEvidence::from_value(
                    serde_json::json!({"operation": "test.store", "injected": "CorruptPhysicalState"}),
                ))
            })?;
            let probe = probe_sequence
                .and_then(|sequence| self.0.get(sequence.checked_sub(1)? as usize))
                .map(|bytes| Arc::from(bytes.as_slice()));
            LoadedRun::new(
                head,
                Arc::from(self.0[0].as_slice()),
                Arc::from(latest.as_slice()),
                probe,
            )
            .map(Some)
        })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async {
            Err(StoreError::CorruptPhysicalState(
                mfm_values::DiagnosticEvidence::from_value(
                    serde_json::json!({"operation": "test.store", "injected": "CorruptPhysicalState"}),
                ),
            ))
        })
    }
}

pub(super) struct FaultStore {
    pub(super) failure: StoreError,
}

impl Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        _run_id: &'a RunId,
        _probe_sequence: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Option<LoadedRun>, StoreError>> + Send + 'a>>
    {
        Box::pin(async move { Err(self.failure.clone()) })
    }

    fn append_run<'a>(
        &'a self,
        _frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = Result<AppendResult, StoreError>> + Send + 'a>> {
        Box::pin(async move { Err(self.failure.clone()) })
    }
}
