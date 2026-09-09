use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc as StdArc, Barrier};
use std::task::{Context, Poll, Waker};

use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId};

use super::*;

#[path = "../tests/support/hostile.rs"]
mod hostile;

fn run(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn reference(name: &str, bytes: &[u8]) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        raw_content_digest(bytes),
    )
    .expect("reference")
}

fn genesis(run: &RunId) -> EncodedRunFrame {
    let program = b"{}";
    let context = b"[]";
    EncodedRunFrame::admission(
        run,
        &reference("mfm.test.program", program),
        program,
        &reference("mfm.test.context", context),
        context,
    )
    .expect("genesis")
}

async fn install_run(store: &MemoryStore, run_id: &RunId, run: MemoryRun) {
    store
        .runs
        .lock()
        .await
        .insert(run_id.clone(), Arc::new(Mutex::new(run)));
}

fn observe<T>(result: std::result::Result<T, StoreError>) -> hostile::Observation {
    match result {
        Ok(_) => panic!("expected Store error"),
        Err(
            StoreError::FrameSize(_)
            | StoreError::HistorySize(_)
            | StoreError::FrameCount(_)
            | StoreError::ArithmeticOverflow,
        ) => hostile::Observation::Capacity,
        Err(StoreError::CorruptPhysicalState) => hostile::Observation::Corrupt,
        Err(StoreError::Unavailable) => hostile::Observation::Unavailable,
        Err(StoreError::Indeterminate) => panic!("Memory must not manufacture Indeterminate"),
    }
}

#[tokio::test]
async fn memory_runs_the_shared_hostile_conformance_matrix() {
    use hostile::{HostileCase as Case, Observation};

    let mut observed = Vec::new();
    let absent = MemoryStore::new();
    observed.push((
        Case::Absence,
        if absent.load_run(&run(40)).await.expect("absent").is_none() {
            Observation::None
        } else {
            panic!("absent Memory run returned a transfer")
        },
    ));

    let orphan_store = MemoryStore::new();
    let orphan_id = run(41);
    let orphan = genesis(&orphan_id);
    install_run(
        &orphan_store,
        &orphan_id,
        MemoryRun {
            frames: vec![Arc::new(StoredFrame {
                bytes: orphan.canonical_bytes().to_vec(),
                head_digest: orphan.head_digest().clone(),
            })],
            head: None,
        },
    )
    .await;
    observed.push((
        Case::AbsentHeadOrphan,
        observe(orphan_store.load_run(&orphan_id).await),
    ));

    let target_store = MemoryStore::new();
    let target_id = run(42);
    let target = genesis(&target_id);
    let mut target_bytes = target.canonical_bytes().to_vec();
    target_bytes[0] ^= 1;
    install_run(
        &target_store,
        &target_id,
        MemoryRun {
            frames: vec![Arc::new(StoredFrame {
                bytes: target_bytes,
                head_digest: target.head_digest().clone(),
            })],
            head: Some(Head {
                sequence: 1,
                total_bytes: target.canonical_bytes().len() as u64,
            }),
        },
    )
    .await;
    observed.push((
        Case::CorruptTarget,
        observe(target_store.append_run(&target).await),
    ));

    let head_store = MemoryStore::new();
    let head_id = run(43);
    let head_frame = genesis(&head_id);
    install_run(
        &head_store,
        &head_id,
        MemoryRun {
            frames: vec![Arc::new(StoredFrame {
                bytes: head_frame.canonical_bytes().to_vec(),
                head_digest: head_frame.head_digest().clone(),
            })],
            head: Some(Head {
                sequence: 2,
                total_bytes: head_frame.canonical_bytes().len() as u64,
            }),
        },
    )
    .await;
    observed.push((
        Case::CorruptHead,
        observe(head_store.load_run(&head_id).await),
    ));

    for (case, byte, corrupt_bytes, corrupt_digest, total_delta) in [
        (Case::CorruptBytes, 44, true, false, 0),
        (Case::CorruptDigest, 45, false, true, 0),
        (Case::CorruptTotal, 46, false, false, 1),
    ] {
        let store = MemoryStore::new();
        let run_id = run(byte);
        let frame = genesis(&run_id);
        let mut bytes = frame.canonical_bytes().to_vec();
        if corrupt_bytes {
            bytes[0] ^= 1;
        }
        let digest = if corrupt_digest {
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([8; 32]))
        } else {
            frame.head_digest().clone()
        };
        install_run(
            &store,
            &run_id,
            MemoryRun {
                frames: vec![Arc::new(StoredFrame {
                    bytes,
                    head_digest: digest,
                })],
                head: Some(Head {
                    sequence: 1,
                    total_bytes: frame.canonical_bytes().len() as u64 + total_delta,
                }),
            },
        )
        .await;
        observed.push((case, observe(store.load_run(&run_id).await)));
    }

    let capacity_id = run(47);
    let capacity_frame = genesis(&capacity_id);
    let stored = Arc::new(StoredFrame {
        bytes: capacity_frame.canonical_bytes().to_vec(),
        head_digest: capacity_frame.head_digest().clone(),
    });
    observed.push((
        Case::FrameCapacity,
        observe(plan_append(
            None,
            0,
            None,
            None,
            Candidate {
                sequence: 1,
                predecessor: None,
                head_digest: capacity_frame.head_digest().clone(),
                bytes: vec![0; MAX_FRAME_BYTES + 1],
            },
        )),
    ));
    observed.push((
        Case::CountCapacity,
        observe(plan_append(
            Some(Head {
                sequence: MAX_RUN_FRAMES,
                total_bytes: capacity_frame.canonical_bytes().len() as u64,
            }),
            MAX_RUN_FRAMES as usize,
            Some(Arc::clone(&stored)),
            None,
            Candidate {
                sequence: MAX_RUN_FRAMES + 1,
                predecessor: Some(capacity_frame.head_digest().clone()),
                head_digest: capacity_frame.head_digest().clone(),
                bytes: capacity_frame.canonical_bytes().to_vec(),
            },
        )),
    ));
    observed.push((
        Case::RunCapacity,
        observe(plan_append(
            Some(Head {
                sequence: 1,
                total_bytes: MAX_RUN_BYTES - capacity_frame.canonical_bytes().len() as u64 + 1,
            }),
            1,
            Some(Arc::clone(&stored)),
            None,
            Candidate {
                sequence: 2,
                predecessor: Some(capacity_frame.head_digest().clone()),
                head_digest: capacity_frame.head_digest().clone(),
                bytes: capacity_frame.canonical_bytes().to_vec(),
            },
        )),
    ));

    let mut publication = MemoryRun {
        frames: Vec::new(),
        head: None,
    };
    observed.push((
        Case::AtomicFault,
        observe(publish_insert(
            &mut publication,
            StoredFrame {
                bytes: capacity_frame.canonical_bytes().to_vec(),
                head_digest: capacity_frame.head_digest().clone(),
            },
            Head {
                sequence: 1,
                total_bytes: capacity_frame.canonical_bytes().len() as u64,
            },
            |_| Err(StoreError::Unavailable),
        )),
    ));
    assert!(publication.frames.is_empty() && publication.head.is_none());

    let retry = plan_append(
        Some(Head {
            sequence: MAX_RUN_FRAMES,
            total_bytes: MAX_RUN_BYTES,
        }),
        MAX_RUN_FRAMES as usize,
        Some(Arc::clone(&stored)),
        Some(stored),
        Candidate {
            sequence: 1,
            predecessor: None,
            head_digest: capacity_frame.head_digest().clone(),
            bytes: capacity_frame.canonical_bytes().to_vec(),
        },
    );
    observed.push((
        Case::NotInsertedBypass,
        match retry {
            Ok(AppendPlan::NotInserted) => Observation::NotInserted,
            _ => panic!("exact retry did not bypass capacity"),
        },
    ));

    hostile::assert_hostile_matrix(&observed);
}

#[test]
fn absent_and_private_empty_loads_are_unavailable_without_tokio() {
    let absent = MemoryStore::new();
    let empty_id = run(28);
    let empty = MemoryStore {
        runs: Mutex::new(BTreeMap::from([(
            empty_id.clone(),
            Arc::new(Mutex::new(MemoryRun {
                frames: Vec::new(),
                head: None,
            })),
        )])),
    };
    for (store, run_id) in [(&absent, run(29)), (&empty, empty_id)] {
        let mut future = store.load_run(&run_id);
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Err(StoreError::Unavailable))
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discarded_or_failed_blocking_jobs_publish_nothing() {
    assert_eq!(
        run_pure_blocking::<(), _>(|| panic!("test-only panic")).await,
        Err(StoreError::Unavailable)
    );

    let entered = StdArc::new(Barrier::new(2));
    let release = StdArc::new(Barrier::new(2));
    let published = StdArc::new(AtomicBool::new(false));
    let task = {
        let entered = StdArc::clone(&entered);
        let release = StdArc::clone(&release);
        let published = StdArc::clone(&published);
        tokio::spawn(async move {
            let result = run_pure_blocking(move || {
                entered.wait();
                release.wait();
                Ok(())
            })
            .await;
            if result.is_ok() {
                published.store(true, Ordering::SeqCst);
            }
        })
    };
    entered.wait();
    task.abort();
    release.wait();
    assert!(task.await.expect_err("aborted").is_cancelled());
    assert!(!published.load(Ordering::SeqCst));
}
