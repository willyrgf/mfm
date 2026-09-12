# mfm-journal

Journal seals and decodes the exact `mfm.run.frame.v6` canonical envelope. Its fields are `domain`,
`run_id`, `run_sequence`, `previous_head_digest`, and the caller's opaque `payload`. The head is
SHA-256 over the exact canonical frame bytes. Sealing and decoding check local header fields and
canonical syntax; old domains, unknown envelope fields, and noncanonical bytes are rejected.

`EncodedRunFrame` exposes immutable checked header fields, exact bytes, and the canonical payload.
Journal has no object table, lifecycle records, Effect pairing state, or history reconstruction.
Runtime owns the current-state payload, typed value admission, operation facts, and recovery rules.
Store owns bounded snapshot loading and atomic exact-head append.

The existing format ceilings remain: 134,283,264 complete frame bytes, 65,536 frames, and 512 MiB
cumulative frame bytes. Runtime checks its 65,536-byte non-payload metadata ceiling against actual
encoded frames. Values owns the 32 MiB canonical object ceiling. Store enforces cumulative append
limits using maintained physical accounting. No prospective history reservation is required.
