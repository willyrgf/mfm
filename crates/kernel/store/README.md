# mfm-store

Store is the object-safe mechanical boundary:

```text
load_run(&RunId) -> Result<Option<StoredRunBytes>, StoreError>
append_run(&EncodedRunFrame) -> Result<AppendResult, StoreError>
```

`MemoryStore` provides complete-prefix snapshots and atomic exact-head append. Exact retry, including
a historical frame, returns `NotInserted`; a competing candidate or orphan writes nothing. Physical
digest/count/byte corruption is rejected through Journal's frame-head helper.

Large candidate/snapshot copies are cooperative; pure validation uses immediately awaited blocking
jobs without mutation authority. Store has no Program/domain/capability/reducer dependency or
semantic facade.
