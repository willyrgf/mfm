# mfm-store

Store is the object-safe mechanical boundary:

```text
load_run(&RunId, Option<u64>) -> Result<Option<LoadedRun>, StoreError>
append_run(&EncodedRunFrame) -> Result<AppendResult, StoreError>
```

`MemoryStore` loads admission, latest, and an optional requested sequence under one lock. Selections
of the same row share immutable bytes. Loads check the bounded selected rows and append-maintained
head accounting; they do not scan or sum prior history. A missing requested row at or below the head
is corruption, while an absent row above the head is a valid snapshot observation.

Append remains atomic and exact-head checked. Exact retry, including a historical frame, returns
`NotInserted`; a competing candidate or orphan writes nothing. Selected physical row digests are
checked through Journal's frame-head helper without decoding payloads.

The sibling `RunIndex` port exposes ascending `RunId` keyset pages over the four mechanical head
fields. The same concrete backend may implement both ports, while `dyn Store` still has no
enumeration authority and listing never parses frames or derives run status.

Large candidate copies are cooperative; snapshots share bounded immutable rows. Pure validation
uses immediately awaited blocking jobs without mutation authority. Store has no Program/domain/capability/reducer dependency or
semantic facade.
