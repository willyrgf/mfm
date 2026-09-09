use mfm_ids::RunId;
use mfm_journal::{EncodedRunFrame, StoredRunBytes};
use mfm_store::{AppendResult, MemoryStore, Store, StoreError};
use std::{collections::VecDeque, future::Future, pin::Pin};

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
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(run_id)
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
                Some(AppendAction::Indeterminate) => Err(StoreError::Indeterminate),
                Some(AppendAction::RetainThenIndeterminate) => {
                    let result = self.inner.append_run(frame).await?;
                    self.record_if_inserted(frame, result);
                    Err(StoreError::Indeterminate)
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
