# mfm-runtime

Runtime owns immutable live assembly, affine sessions, direct-new `CommittedCall`, typed handoff,
the Runtime-owned affine `PendingConclusion`, and the exhaustive `SuspendedRun` owner-fate
coordinator around Store. Provider entry requires a consuming call token. Access registration
retains the immutable binding descriptor, and conclusion recovery retains no provider or callback
authority. Integrity-blocked Access outcomes use the declared static failure contract and do not
invoke State interpretation.
