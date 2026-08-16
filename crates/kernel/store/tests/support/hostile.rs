#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HostileCase {
    Absence,
    AbsentHeadOrphan,
    CorruptTarget,
    CorruptHead,
    CorruptBytes,
    CorruptDigest,
    CorruptTotal,
    FrameCapacity,
    CountCapacity,
    RunCapacity,
    AtomicFault,
    NotInsertedBypass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    None,
    NotInserted,
    Capacity,
    Corrupt,
    Unavailable,
}

pub fn assert_hostile_matrix(observed: &[(HostileCase, Observation)]) {
    use HostileCase::*;
    use Observation::*;

    let expected = [
        (Absence, None),
        (AbsentHeadOrphan, Corrupt),
        (CorruptTarget, Corrupt),
        (CorruptHead, Corrupt),
        (CorruptBytes, Corrupt),
        (CorruptDigest, Corrupt),
        (CorruptTotal, Corrupt),
        (FrameCapacity, Capacity),
        (CountCapacity, Capacity),
        (RunCapacity, Capacity),
        (AtomicFault, Unavailable),
        (NotInsertedBypass, NotInserted),
    ];
    let mut observed = observed.to_vec();
    observed.sort_by_key(|(case, _)| *case);
    assert_eq!(observed, expected);
}
