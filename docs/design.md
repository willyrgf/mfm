# Design

MFM durably proves a caller-driven execution of one immutable typed Program. A Program contains an
ordered array of State or Match declarations. Index zero is the root; every successor is a forward
`u16` index, and array order is the only control identity. A State has exact input, output, and
failure contracts. Missing success/failure successors mean the corresponding exact root result;
`Never` is the reserved uninhabited failure contract and can never be encoded as a value.

Source code authors the graph through one deterministic typed `Operation`. `expand_program` gives
the root Operation the sole `OperationExpansion` compiler context, which flattens Pure States,
exact-pair Reads, child Operations, structured Match joins, and exact failure handlers into one
private symbolic draft before constructing Program v2. Operations, callbacks, injection setup, and
scope boundaries are erased; only the immutable State/Match graph is persisted.

Pure States deterministically map typed input to typed success/failure. Read States deterministically
prepare typed intent, then interpret typed evidence. Runtime associates all values, State drivers,
Match projections, and `(capability contract, binding ref)` adapters before execution. State code has
no ambient IO and there is no generic effect or mutation capability.

Each Read occurrence applies the exact capability/State pair's authoring-time injection policy.
Injection may add deterministic Pure topology before or after the one kernel-owned Read, but it
cannot perform IO, access or replace that Read, register an adapter, or grant execution authority.
Effect, signing, nonce, broadcast, and durable mutation authority remain deferred.

Runtime admits `(RunId, Program, C0)`, appends genesis, folds the qualified history, executes only
the selected declaration, and appends one fused conclusion. A Read frame contains intent, accepted
evidence, and outcome together; adapter errors append nothing. Match is a pure projection and adds no
frame. Zero-State Programs terminate at genesis. Hot advancement and cold reload use the same fold.

Journal owns the exact `mfm.run.frame.v1` canonical wire, recursive exact-byte SHA-256 heads, strict
frame-local object closure, and history qualification. Store sees only sealed frames and opaque
complete transfers. It atomically inserts at the exact head or writes nothing.

The fixed limits are 8 MiB per canonical run object, 65,536 non-payload envelope bytes, 25,231,360
bytes per frame, 65,536 frames, and 512 MiB of frame bytes per run. These are format bounds, not
tunable runtime policy.

PostgreSQL is a fresh three-table baseline: `mfm_store_schema`, `mfm_run_frames`, and
`mfm_run_heads`. Connection admission checks exact schema shape, logged tables, primary status,
`fsync`, and `full_page_writes`. Loads use one read-only repeatable snapshot. Appends take the
per-RunId advisory transaction lock before observing state and force synchronous COMMIT. Schema
provisioning is one idempotent storage-crate entry that installs only into an empty store: it
verifies an existing installation, refuses to touch any incompatible one, and never migrates.

EVM physical route identity is the domain-owned, secret-free `EvmPhysicalTarget { chain_id,
endpoint_ref }`. Planning and adapter registration derive the same content ref. Credentials and
client handles are process-local and never persisted. Wrong local route/chain is `Internal` before
provider entry; only authenticated external evidence may become `IntegrityBlocked`.

Application supports Portfolio snapshot planning/start plus Runtime resume/read. Callers provide the
RunId explicitly. Transaction submission requires a future durable transaction-authority/outbox
design and is not part of this system.

Secrets do not enter Program, C0, frames, Store metadata, RunView, outputs, logs, or error details.
Writable restoration behind acknowledged state is unsupported; a new writable timeline requires
fresh external authority and fresh RunIds.
