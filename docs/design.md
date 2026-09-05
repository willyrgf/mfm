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
complete command and interpret typed settlement evidence. Runtime associates all values,
mode-specific State executables, Match projections, and exact `(capability contract, binding ref)`
adapters before execution. A semantic type ID names a value family and may have multiple exact
generic schemas; only the exact content ref selects a codec. State association likewise uses the
exact implementation/input/output/failure ABI, so one implementation identity may own multiple
generic ABIs without ambiguity. Read callbacks and evidence binding receive the exact intent value
ref; Effect callbacks receive the exact command value ref used for `EffectId` derivation. These are
instance refs, never shared codec contract refs. State code has no ambient IO.

Each Read or Effect occurrence applies the exact capability/State pair's authoring-time injection
policy. Before and after hooks use typed `OperationExpansion` scopes and the same Pure, Read,
Effect, child Operation, Match, and failure-handler authoring as Operations. The kernel inserts the
designated occurrence once between them. Hooks perform no IO or adapter registration and have no
handle for modifying the designated occurrence. Nested hooks share the existing callback-depth and
graph bounds. Empty hooks require identical input/output contracts; local successful scope exits
rejoin the designated occurrence or caller continuation. The complete expansion owns its input,
output, and failure contracts. Original failures skip the after hook: it is a success continuation,
not a finally handler. Every suffix is checked completely before merging into its caller.

Runtime admits `(RunId, Program, C0)`, appends genesis, folds the qualified history, and executes only
the selected declaration. Pure and Read append one fused conclusion; a Read frame contains intent,
accepted evidence, and outcome together. Effect execution first appends the complete command and
derived `EffectId`, and enters the adapter only after known insertion. `Settled(evidence)` binds and
interprets the evidence before appending the adjacent conclusion. `Pending` retains the identical
prepare, appends nothing, and returns a Runnable view without re-entering the adapter in that
invocation. An adapter error also leaves the prepare pending and appends no conclusion, but returns
its distinct Runtime error. Match is a pure projection and adds no frame. Zero-State Programs
terminate at genesis. Hot advancement and cold reload use the same fold. Cold fold re-prepares only
a retained Effect command to validate its exact bytes and identity; retained Pure/Read conclusions
remain authoritative event-log outcomes.

Journal owns the exact `mfm.run.frame.v2` canonical wire, recursive exact-byte SHA-256 heads, strict
frame-local object closure, Effect prepare/conclusion adjacency, and history qualification. Store
sees only sealed frames and opaque complete transfers. It atomically inserts at the exact head or
writes nothing and has no Effect semantics.

The fixed limits are 8 MiB per canonical run object, 65,536 non-payload envelope bytes, 25,231,360
bytes per frame, 65,536 frames, and 512 MiB of frame bytes per run. Program validates the
conservative bound `1 + Pure + Read + 2*Effect <= 65,536` across every declaration. These are format
bounds, not tunable runtime policy.

Signing owns the checked transient recoverable-secp256k1 public key, 32-byte digest, low-S compact
recoverable signature, public recovery, and key- and purpose-bound `Secp256k1Signer` contract.
The in-process keystore constructs its non-`Send`, non-`Sync` key map inside one dedicated owner
thread, accepts only checked zeroizing scalars, retains at most 64 distinct key instances, and
communicates over a bounded channel. Purpose is immutable on each returned handle and does not cross
the owner channel. Duplicate same-key imports return handles with the same public key and may
carry different purposes without consuming another key slot. Explicit async shutdown requests exit
and immediately awaits an OS-thread join in blocking work; dropping the final sender also ends the
owner loop.

EVM Program values use byte-backed checked lowercase `EvmAddress` and `EvmHash`, canonical decimal
`EvmU256`, exact `CanonicalBytes`, and `NonZeroU64` chain IDs and gas limits. A transaction binding
fixes chain ID plus expected genesis hash, endpoint reference, authority epoch, and sender account.
The sole transaction command is nonce-free EIP-1559 type 2 with an empty access list and a private
bounded Create or Call action. Its complete factories and checked deserializer enforce input bounds,
the `u128` fee ceiling, priority-fee ordering, and nonzero gas. One generic
`ExecuteEvmTransaction<K>` Effect State prepares the shared `EvmTransactionEffect` command
unchanged. Its completion retains caller context, binding, a shared receipt, and only the created
address or checked call target; its reversion retains caller context and the same receipt.
Settlement evidence binds the pending EffectId and contains the reserved nonce, shared receipt, and
one Created, Called, or Reverted outcome. The Effect binder rejects an opposite successful action
before State interpretation.

The broad EVM Read intent fixes only a nonzero chain ID, physical-route content ref, and one of six
balance subjects; that subject is the operation discriminator. Its returned sum contains checked
chain IDs, anchors, raw units, or token decimals bounded to 0 through 30. The context-preserving
anchored contract-call Read has separate intent and evidence types fixing a transaction-route
content ref, target, bounded calldata, and exact block anchor. Every broad and anchored evidence
outcome carries Runtime's exact qualified intent value ref, which the capability binder checks
before its typed subject/result relationship. Returned anchored evidence additionally contains the
same anchor and bounded return bytes. Rejected, safe-failure, and integrity-blocked evidence project
to a closed failure reason. Caller-owned Pure States project transaction completions into subsequent checked
commands and observations; EVM does not own action-specific bridges or a product lifecycle
Operation. These domain contracts perform no provider, signing, nonce, or persistence IO; the
authority and live adapter remain separate downstream responsibilities.

The live EVM transaction adapter captures one exact binding, key- and purpose-bound signer,
append-only transaction authority, and transaction-only provider facet. Registration validates the
immutable authority epoch, purpose `mfm.evm.sign-eip1559@1`, and public-key-derived sender once.
Before per-invocation authority or provider IO it compares the complete command binding and exact
command value ref. It reserves one provider-observed
pending nonce, signs the fixed type-2 payload through that handle, retains exact signed bytes, and
reconciles receipts before retaining typed settlement. Retained bytes are fully decoded and
compared before provider entry. A null receipt causes at most one submission of those bytes in an
invocation; a matching submission response returns normal Pending progress, while a transport
failure, dropped acknowledgement, malformed response, or hash mismatch remains Unavailable.
Settlement requires one validated receipt and equality with the provider's current canonical
identity for its block number. Version 1 is the canonical-receipt policy for
the pinned non-reorging development fixture and is not registered by production composition.

The same JSON-RPC client implements a separate transaction provider facet and the generic anchored
contract-call observation. Anchored calls re-observe the authored block by number, require deployed
code at its canonical block-hash selector, perform one call at that selector, and re-observe the
same block before returning bounded bytes. An absent named block is SafeFailure, no code is
Rejected, and a replaced authenticated anchor is IntegrityBlocked; RPC and malformed-ingress
failures remain Unavailable. The observational provider boundary receives checked broad or anchored
intents plus Runtime's exact canonical intent value ref. Its bounded generic JSON-RPC ingress uses
typed parameters and exact success/failure envelopes; every RPC error is Unavailable, while empty
broad token-call data alone retains the missing-interface SafeFailure policy.

PostgreSQL has three fresh baselines behind two independently gated handles. Run history owns
`mfm_store_schema`, `mfm_run_frames`, and `mfm_run_heads` in `public`; opaque versioned
configuration custody owns `mfm_config_schema` and `config_revisions` in `mfm_config`; and EVM
transaction authority owns exactly its marker plus three append-only stage tables in `mfm_evm_tx`.
Run heads
also provide the mechanical RunIndex projection without parsing frames. `PostgresBackend` gates
only run history and configuration; `PostgresEvmTransactionAuthority` owns a separate pool and
gates only its optional schema. Every admission checks its owned exact catalog, privileges, primary
status, `fsync`, and `full_page_writes`. One declarative catalog specification and normalized
verifier owns run-history, configuration, and EVM admission, including expanded column ACLs,
`MAINTAIN`, RLS/policies/rules/triggers, storage options and tablespaces, validated constraints, and
live/ready/valid indexes.

PostgreSQL data reads and deletes target schema-qualified `ONLY` relations. Inserts and conflict
updates target the named physical table. Inherited descendants outside the owned schemas are
outside MFM custody: their rows cannot affect markers, history, configuration, or transaction
authority, and exact configuration deletion cannot remove their rows. Admission continues to
qualify the owned objects; it does not police unrelated inheritance descendants.


Loads use one read-only repeatable snapshot. Appends take the per-RunId advisory transaction lock
before observing state and force synchronous COMMIT. Configuration imports use the `(name, digest)`
primary key to create or compare immutable revisions. Import and exact idempotent delete force
synchronous COMMIT and preserve ambiguous acknowledgement. There is no revision-count limit. A
base and optional authority provisioners accept separate typed admin/runtime locators for one
normalized target, install or verify only their owned surfaces, and grant the fixed `mfm_runtime`
role exact DML authority. Production invokes only the base provisioner.
Runtime connections have no ownership or DDL authority. Existing installations are verified and
never migrated, repaired, re-owned, or reset.

The transaction authority epoch is generated with OS cryptographic entropy in the fresh schema
transaction and captured by the direct admission gate. Every pooled connection repeats the full
gate and must observe that epoch. A nonce domain is exactly epoch, chain instance, and sender;
custody-provider and endpoint identities are not nonce dimensions. Reservation serializes one
domain with a mechanical advisory lock and accepts provider pending nonce only at initial creation
or exact authority-next. Reservations, prepared raw transactions, and terminal settlements are
immutable insert-or-compare facts. One marker-driven load statement reconstructs the complete
optional fact and rejects a wrong epoch without hiding the reservation. Different concurrently
qualified Prepared or Settled candidates return `Unavailable` so the retained first winner is
qualified on reload.
Settlement bytes are current-type canonical JSON and must exactly match EffectId, command-selected
reservation, nonce, and prepared transaction hash. Ambiguous authority COMMIT acknowledgement is
`Unavailable`; a later `load` qualifies the committed-or-absent outcome. Writable rollback of an
epoch is unsupported.

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
The generic Effect protocol supplies durable command authorization and caller-driven recovery. The
development EVM transaction adapter deliberately remains outside production composition: a future
irreversible production finality policy requires a new capability contract identity rather than
reusing canonical-receipt evidence.

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
