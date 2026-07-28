# MFM durable executor reference storage

This crate is the durable local-filesystem implementation used to qualify the generic
`mfm-executor` protocol before PostgreSQL integration. It is a conformance and fault-injection
backend, not a selectable production authority.

`FileExecutorStore::open` receives one strict `VerifiedExecutorBinding` and returns the asynchronous
raw store used by `KeyedExecutorLedger<FileExecutorStore>`. Immutable evidence bounds come only from
that binding's executor contract descriptor, so a reopen cannot substitute a second bounds
authority. The file backend owns persistence and atomic compare-and-append only; the shared engine
owns folding, resource policy, attempts, observations, and tombstones.

Each acknowledged transition:

1. takes an exclusive cross-process file lock;
2. reloads and verifies the greatest immutable snapshot;
3. applies one in-memory append/CAS transaction;
4. writes and `fsync`s a checksummed temporary snapshot;
5. atomically publishes it with a same-filesystem hard link;
6. `fsync`s the containing directory; and
7. releases the lock before any executor-to-destination call.

Snapshots are append-only generations. A corrupt greatest snapshot fails closed; the backend never
falls back to an older generation. Crash before publication leaves no committed transition. Crash
after publication but before acknowledgement returns `OutcomeUnknown` for a ledger append, never
affine target authority. It may leave an unmatched durable authorization or destination result,
which the convergence protocol handles on a fresh load and derivation.

The backend rejects symlink and non-regular directory, lock, and snapshot targets. Its deterministic
fault points exercise failure before publication and acknowledgement ambiguity after publication;
neither path can return an authority or receipt whose matching snapshot was not durably published.

This backend proves crash/reopen and cooperating multi-process behavior on a filesystem that
honors file locks, hard-link atomicity, file `fsync`, and directory `fsync`. It does **not** make a
filesystem backup non-rollback. Removing a valid suffix or restoring the whole directory can only
be prevented by a non-rollback generation service or an authoritative destination fence outside
that restore domain. Therefore it does not, by itself, close production split-brain or backup
rollback qualification.
