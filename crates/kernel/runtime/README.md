# mfm-runtime

RuntimeAssemblyBuilder registers typed values, Pure/Read States, Match descriptors, and Read
adapters. `finish` produces one immutable assembly. Program association pre-resolves all exact
contracts and callbacks before execution.

Runtime owns `start`, `resume`, `read`, and the sole hot/cold semantic fold. It appends genesis and
fused conclusions through Store, extends locally after `Inserted`, and completely reloads after
`NotInserted` or on cold entry. Match is a no-frame structural projection. `read` never progresses.
The Match projection selects and qualifies the exact nested canonical payload bytes through the
registered payload codec; it never serializes a typed selector or payload.

Adapter errors map to redaction-safe Runtime errors and append nothing. Dropping at any await is
safety-neutral; Runtime has no background finalizer, pending owner, semaphore, timeout, retry token,
or public State-by-State lifecycle.
