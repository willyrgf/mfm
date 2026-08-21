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

PostgreSQL has two independent fresh baselines. Run history owns `mfm_store_schema`,
`mfm_run_frames`, and `mfm_run_heads` in `public`; opaque named config custody owns
`mfm_catalog_schema` and `config_entries` in `mfm_catalog`. Run heads also provide the mechanical
RunIndex projection without parsing frames. Each connection admission gate checks only its schema's
exact logged relations, constraints, ownership and privileges, plus primary status, `fsync`, and
`full_page_writes`.

Loads use one read-only repeatable snapshot. Appends take the per-RunId advisory transaction lock
before observing state and force synchronous COMMIT. Catalog mutations take one global advisory
transaction lock, enforce 256 live entries, and preserve ambiguous COMMIT acknowledgement. A
short-lived admin provisioner accepts separate typed admin/runtime locators for one normalized
target, installs only absent namespaces, and grants the fixed `mfm_runtime` role exact DML authority.
Runtime connections have no ownership or DDL authority. Existing installations are verified and
never migrated, repaired, re-owned, or reset.

PostgreSQL network authority is one strict URI with an explicit password, numeric `127.0.0.1` or
`::1` host, and `sslmode=disable`. PostgreSQL traffic is intentionally plaintext inside the trusted
shared network namespace; remote database targets are unsupported. The private locator uses stock
SQLx, overwrites every ambient-derived value that can affect this plaintext connection, and rejects
`PGOPTIONS`, whose startup effects SQLx cannot clear. Home/passfile and service-file inputs cannot
influence the resulting authority.

EVM physical route identity is the domain-owned, secret-free `EvmPhysicalTarget { chain_id,
endpoint_ref }`. Planning and adapter registration derive the same content ref. Credentials and
client handles are process-local and never persisted. Wrong local route/chain is `Internal` before
provider entry; only authenticated external evidence may become `IntegrityBlocked`.

Live composition accepts an empty or strictly sorted, unique set of at most 256 EVM bindings.
Multiple endpoints may bind one chain. One `ComposedRuntime` derives its immutable assembly,
planning targets, public `(chain_id, endpoint_id, binding_ref)` views, Store, and RunIndex from that
single checked input and concrete backend; there is no singular one-route assembly constructor.

Application owns the transport-neutral client surface. A strict XDG/HOME- or override-selected
`deployment.toml` names environment resolvers for the runtime PostgreSQL locator and stable public
EVM bindings; it contains no locator values and has no product lifecycle. Production composition
resolves each raw private locator once, constructs local PostgreSQL and stock EVM HTTP(S) clients,
and derives Runtime registrations, planning targets, public binding views, Store, and RunIndex from
the same checked binding set. The EVM client uses no ambient proxy, redirect, referer propagation, or
automatic retry.

The named config catalog retains complete, bounded canonical config documents tagged by the exact
entry point. Import validates and plans the document before atomic insert or replacement. Run start selects
either the name's current revision or an exact `sha256-jcs-v1` revision, checks every requested
binding before Runtime Store IO, and returns the selected config summary with the run view. The
shared surface also owns entry-point/binding discovery, config read/list, run
progress/read, and mechanical run-head listing. Every execution receives an explicit caller-owned
RunId; ambiguous append acknowledgement carries the exact start or progress recovery identity.
Transaction submission requires a future durable transaction-authority/outbox design and is not
part of this system.

CLI and REST are thin renderings of that single surface and add no authentication or authorization
layer. REST serves HTTP/1 on one caller-selected Unix socket; the enclosing deployment owns access
isolation, permissions, and stale-socket cleanup. Bounded bodies and queries are REST transport
concerns. Schema provisioning and optional RunId generation remain CLI-only; REST requires the
caller's RunId in the path and exposes no administrative or secret-custody route.

Secrets do not enter Program, C0, frames, Store metadata, RunView, outputs, logs, or error details.
Writable restoration behind acknowledged state is unsupported; a new writable timeline requires
fresh external authority and fresh RunIds.

## Material uncertainties

none
