# mfm-journal

Strict persisted data and identities for structured Runtime history.

The only record families are:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

The crate also owns content-addressed `HistoryObject`, typed/lexical value references, facts,
record/head coordinates, assigned batches, semantic heads, and closed access outcomes. Prior-run
fact data includes the strict admitted source manifest, tenant publication/barrier coordinate,
scanner certificate, selected-source provenance with exact subject/response/claim bytes, and
completeness attestation. Every persisted
struct denies unknown fields and uses canonical float-free encodings.

It does not persist, fold, schedule, execute callbacks, or define a storage backend. Successor
legality and exact object closure belong to `mfm-store`.
