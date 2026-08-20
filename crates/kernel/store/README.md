# mfm-store

Store is the object-safe mechanical boundary:

```text
load_run(&RunId) -> Result<Option<StoredRunBytes>, StoreError>
append_run(&EncodedRunFrame) -> Result<AppendResult, StoreError>
```

`MemoryStore` provides complete-prefix snapshots and atomic exact-head append. Exact retry, including
a historical frame, returns `NotInserted`; a competing candidate or orphan writes nothing. Physical
digest/count/byte corruption is rejected through Journal's frame-head helper.

The same concrete backend separately implements `mfm_catalog::RunIndex` with ascending `RunId`
keyset pages over the four mechanical head fields. A `dyn Store` still has no enumeration authority,
and listing never parses frames or derives run status.

Large candidate/snapshot copies are cooperative; pure validation uses immediately awaited blocking
jobs without mutation authority. Store has no Program/domain/capability/reducer dependency or
semantic facade.
