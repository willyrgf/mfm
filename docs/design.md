# Design

MFM durably proves a caller-driven execution of one immutable typed Program. A Program contains an
ordered array of State or Match declarations. Index zero is the root; every successor is a forward
`u16` index, and array order is the only control identity. A State has exact input, output, and
failure contracts. Missing success/failure successors mean the corresponding exact root result;
`Never` is the reserved uninhabited failure contract and can never be encoded as a value.

Source code authors the graph through one deterministic typed `Operation`. Product-inspection IDs
and descriptions on admitted Operations and States are source metadata only: they are not lowered,
hashed, or persisted. `expand_program` gives
the root Operation the sole `OperationExpansion` compiler context, which flattens Pure States,
exact-pair Reads and Effects, child Operations, structured Match joins, and exact failure handlers
into one private symbolic draft before constructing Program v3. Operations, callbacks, injection
setup, and scope boundaries are erased; only the immutable State/Match graph is persisted.

Pure States deterministically map typed input to typed success/failure. Read States deterministically
prepare typed intent, then interpret typed evidence. Effect States deterministically prepare a
complete command and interpret typed settlement evidence. Runtime associates all values, State
drivers, Match projections, and mode-specific `(capability contract, binding ref)` adapters before
execution. State code has no ambient IO.

Each Read or Effect occurrence applies the exact capability/State pair's authoring-time injection
policy. Injection may add deterministic Pure topology before or after the one kernel-owned
occurrence, but it cannot emit a Read, Effect, child Operation, Match, or failure handler; perform
IO; access or replace the occurrence; register an adapter; or grant execution authority.

Runtime admits `(RunId, Program, C0)`, appends genesis, folds the qualified history, and executes only
the selected declaration. Pure and Read append one fused conclusion; a Read frame contains intent,
accepted evidence, and outcome together. Effect execution first appends the complete command and
derived `EffectId`, enters the adapter only after known insertion, and then appends accepted evidence
and outcome. An adapter error leaves the prepare pending and appends no conclusion. Match is a pure
projection and adds no frame. Zero-State Programs terminate at genesis. Hot advancement and cold
reload use the same fold. Cold fold re-prepares only a retained Effect command to validate its exact
bytes and identity; retained Pure/Read conclusions remain authoritative event-log outcomes.

Journal owns the exact `mfm.run.frame.v2` canonical wire, recursive exact-byte SHA-256 heads, strict
frame-local object closure, Effect prepare/conclusion adjacency, and history qualification. Store
sees only sealed frames and opaque complete transfers. It atomically inserts at the exact head or
writes nothing and has no Effect semantics.

The fixed limits are 8 MiB per canonical run object, 65,536 non-payload envelope bytes, 25,231,360
bytes per frame, 65,536 frames, and 512 MiB of frame bytes per run. Program validates the
conservative bound `1 + Pure + Read + 2*Effect <= 65,536` across every declaration. These are format
bounds, not tunable runtime policy.

Signing owns the exact recoverable-secp256k1 public key identity, 32-byte transient digest,
low-S compact recoverable signature, public recovery, and key-bound `Signer` contract. The
in-process keystore constructs its non-`Send`, non-`Sync` key map inside one dedicated owner thread,
accepts only checked zeroizing scalars, retains at most 64 distinct key instances, and communicates
over a bounded channel. Duplicate import returns another handle with the same public key content
ref. Explicit async shutdown requests exit and immediately awaits an OS-thread join in blocking
work; dropping the final sender also ends the owner loop.

PostgreSQL has two fresh baselines behind one backend, pool, and connection gate. Run history owns
`mfm_store_schema`, `mfm_run_frames`, and `mfm_run_heads` in `public`; opaque versioned
configuration custody owns `mfm_config_schema` and `config_revisions` in `mfm_config`. Run
heads also provide the mechanical RunIndex projection without parsing frames. Every connection
admission checks both schemas' exact logged relations, constraints, ownership and privileges, plus
primary status, `fsync`, and `full_page_writes`; a partially compatible installation is never
exposed.

Loads use one read-only repeatable snapshot. Appends take the per-RunId advisory transaction lock
before observing state and force synchronous COMMIT. Configuration imports use the `(name, digest)`
primary key to create or compare immutable revisions. Import and exact idempotent delete force
synchronous COMMIT and preserve ambiguous acknowledgement. There is no revision-count limit. A
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

The configuration repository retains complete, individually bounded canonical documents tagged by
the exact entry point. Import validates and plans the document before atomically creating or
comparing one immutable revision. Run start requires an exact retained name and `sha256-jcs-v1`
digest, checks every requested binding before Runtime Store IO, and returns the selected config
summary with the run view. The management surface exposes import, complete unpaginated listing, and
idempotent exact delete. Deletion does not revoke runs already admitted from that revision. The shared
surface also owns compiled component, entry-point, and binding discovery, run progress/read, and
mechanical run-head listing. Compiled component discovery is an unexpanded inventory of the public
entry points and reusable Operations admitted by Application composition plus the exact States that
the same composition registers with Runtime.
Every execution receives an explicit transport-selected RunId. Both client surfaces accept one or
use the same Application client primitive to derive one from OS cryptographic entropy before the
start use case. Ambiguous append acknowledgement carries the exact start or progress recovery
identity.
The generic Effect protocol supplies durable command authorization and caller-driven recovery.
Production transaction submission still requires the separate EVM transaction authority and live
adapter introduced by the transaction-specific design; production composition registers no Effect
adapter at this stage.

CLI and REST are thin renderings of that single surface and add no authentication or authorization
layer. REST serves HTTP/1 on one caller-selected Unix socket; the enclosing deployment owns access
isolation, permissions, and stale-socket cleanup. Bounded bodies and queries are REST transport
concerns. Schema provisioning and compiled-component inspection remain CLI-only. Both client
binaries may select an optional RunId or use the shared generation primitive; REST exposes no
administrative, developer-inspection, or secret-custody route.

Secrets do not enter Program, C0, frames, Store metadata, RunView, outputs, logs, or error details.
Writable restoration behind acknowledged state is unsupported; a new writable timeline requires
fresh external authority and fresh RunIds.

## Material uncertainties

none
