# Design

MFM durably proves caller-driven execution of an immutable typed Program. Program contains an
ordered sequence of State declarations with exact input, output and original failure contracts,
selected recovery policies and parameters, explicit root failure maps, scoped checkpoint positions,
finite recovery allowances, and complete frame bounds. Success advances linearly; recovery is a
Runtime transition. `Never` remains the uninhabited failure contract.

Source code authors the sequence through a deterministic `Operation`. Its required `validate_input`
check establishes agreement between root planning assumptions and the initial value before expansion.
`expand_program(entry, operation, input, limits)` qualifies that input and commits its exact value
reference into Program v4. Runtime rejects input substitution before genesis or provider entry;
cold reconstruction checks genesis against the same commitment. Parent planning and deterministic
States establish future child input agreement. No authoring callback enters Runtime. Multiple RunIds
may reuse the same exact Program/input pair.

OperationExpansion lowers Pure/Read/Effect States and child Operations through one private symbolic
draft. Scoped checkpoint tokens cannot be captured for direct installation in another scope, while
inherited installed bindings retain their owning scope for final relocation. Operations, callbacks,
and injection setup remain authoring-only. Program retains only the selected immutable descriptors.

The framework defaults to `NoRecovery` and `Stop`. The shipping Portfolio planner selects those
defaults with zero global and local recovery allowances. Library callers may explicitly install
other policies. EVM owns the selectable `EvmBalanceClassifier` and the checked `AnchorChanged`
cause for bound observations differing from a retained collection anchor; it does not override
caller classification or handler defaults inside `CollectEvmBalances`.

Pure States deterministically map typed input to typed success/failure. Read States deterministically
prepare typed intent, then interpret typed evidence. Effect States deterministically prepare a
complete command and interpret typed settlement evidence. Runtime associates all values,
mode-specific State executables, recovery callbacks, and exact `(capability contract, binding ref)`
adapters before execution. A semantic type ID names a value family and may have multiple exact
generic schemas; only the exact content ref selects a codec. State association likewise uses the
exact implementation/input/output/failure ABI, so one implementation identity may own multiple
generic ABIs without ambiguity. Read callbacks and evidence binding receive the exact intent value
ref; Effect callbacks receive the exact command value ref used for `EffectId` derivation. These are
instance refs, never shared codec contract refs. State code has no ambient IO.

Contexts remain ordinary immutable `MfmValue` snapshots. `ContextSlot<C>` borrows one typed
field or moves its replacement and every unchanged sibling into a new exact value type.
`MfmContext` derives this reconstruction for distinct bare generic fields of a named record;
it introduces no Runtime context service, codec, or scheduler. Slot identity uses an explicit
context namespace and stable field name. Domain stage types and checked fact constructors, not
the mechanical slot primitive, own preservation of preceding facts.

Pure evaluation and Read/Effect interpretation return a proposed domain outcome or the
Program-owned redaction-safe `StateExecutionError`. Runtime maps this internal implementation
failure to `RuntimeError::Internal` before qualifying or appending a conclusion. Pure and Read
errors preserve the current head; an Effect interpretation error preserves its acknowledged
prepare even if external execution has occurred. Retrying uses that same retained command and
EffectId. Cold fold never reinterprets completed conclusions. `PreparationError` remains the
separate internal failure of deterministic intent/command preparation, before adapter entry.
Only authenticated transaction reversion produces a reversion outcome; local action mismatches
are internal execution errors.

Each Read or Effect occurrence applies the exact capability/State pair's authoring-time injection
policy. Before and after hooks use typed `OperationExpansion` scopes with the same linear State,
child Operation, classifier, handler and checkpoint authoring as Operations. The kernel inserts the
designated occurrence once between them. Hooks perform no IO or adapter registration and cannot
modify the designated occurrence. Nested hooks share a 16-level callback bound, counting the root as one, and the declaration
bounds. Before/after hooks are sibling levels. Nested entry is checked before descriptor
construction; suspended occurrence descriptors remain boxed during prefix authoring. Failed
expansions leave the parent draft unchanged. Trusted callbacks compose through OperationExpansion;
this is not a sandbox for unrestricted Rust recursion or stack allocation.
Empty hooks require identical input/output contracts. The complete expansion owns its input,
output, expanded failure contract and explicit designated-to-expanded failure map. Original failures
skip the after hook, which is a success continuation. Every suffix is checked before merging.

Runtime admits `(RunId, Program, C0)`, appends genesis, folds the qualified history, and executes only
the selected declaration. Pure and Read append one fused conclusion; a Read frame contains intent,
accepted evidence and domain outcome, or the original operational error and State-owned context.
The same frame commits any retry, restart or terminal stop. Accepted recovery spends local/global
allowance, advances the visit identity and yields; cold reconstruction restores that exact decision
without invoking a classifier, handler or adapter. Checkpoints retain their active typed input and
restart drops later checkpoint snapshots. An acknowledged Effect prevents restart across its position.

Effect execution first appends the complete command and derived `EffectId`, and enters the adapter
only after known insertion. `Settled(evidence)` binds and interprets evidence before appending the
adjacent conclusion. `Pending` retains the identical prepare, appends nothing and returns an
`EffectPending` view. An operational adapter error also preserves the prepare, returning a stopped
invocation with the last observed view and typed incident. Explicit resume uses the same command
and EffectId. A settled Effect cannot retry or restart. Zero-State Programs terminate at genesis.
Hot advancement, pre-append validation and cold reconstruction use the sole semantic fold.
Pre-append validation constructs a report only for terminal failure candidates. Public observations
share immutable qualified bytes; retaining `last_observed` does not copy the complete admitted input. Cold
fold re-prepares only the final pending Effect command to validate its exact bytes and identity;
completed conclusions remain authoritative event-log outcomes.

Journal owns the `mfm.run.frame.v3` canonical wire, recursive exact-byte SHA-256 heads,
strict frame-local object closure, Effect prepare/conclusion adjacency and history qualification.
Store sees only sealed frames and opaque complete transfers. It atomically inserts at the exact
head or writes nothing and has no Effect semantics.

The fixed limits are 32 MiB per canonical run object, 65,536 non-payload envelope bytes, 134,283,264
bytes per frame, 65,536 frames, and 512 MiB of frame bytes per run. For global recovery limit `G`,
Program bounds frames by `1 + (G + 1) * (Pure + Read + 2*Effect)` and bounds cumulative bytes by
genesis plus `G + 1` copies of the declared complete sequence closure. Each Pure/Read occurrence
bounds its complete conclusion; each Effect bounds prepare and conclusion separately. Bounds cover
all retained canonical objects, root failures and envelopes. Runtime rejects excessive concrete
admission before genesis or provider entry. Format ceilings are not tunable runtime policy.

Signing owns the checked transient recoverable-secp256k1 public key, 32-byte digest, low-S compact
recoverable signature, public recovery, and key- and purpose-bound `Secp256k1Signer` contract.
The in-process keystore constructs its non-`Send`, non-`Sync` key map inside one dedicated owner
thread, accepts only checked zeroizing scalars, retains at most 64 distinct key instances, and
communicates over a bounded channel. Purpose is immutable on each returned handle and does not cross
the owner channel. Duplicate same-key imports return handles with the same public key and may
carry different purposes without consuming another key slot. Explicit async shutdown requests exit
and immediately awaits an OS-thread join in blocking work; dropping the final sender also ends the
owner loop.

Inline failure reports retain original and mapped payloads even when identical. Each report is
limited to 32 MiB independently of its individually qualified cause values. Admission bounds Journal
lifecycles, not the size of every combined report. Report overflow returns `size_limit_exceeded`
before the terminal append, preserving the acknowledged Runnable or EffectPending head and any
pending command authority; repeated explicit progress can encounter the same limit. No report is
persisted separately. Size errors identify the resource, actual size and limit without payload
contents; unrepresentable capacity arithmetic is a distinct invocation error.

The generic canonical JSON syntax ceiling is 256 MiB so complete frames fit; it does not replace
Values' object or Journal's frame limits. Run-history PostgreSQL baseline v2 enforces the larger
frame limit and rejects v1 installations. Provision a fresh development schema; no existing history
or acknowledged head is migrated, rewritten or truncated.

EVM Program values use byte-backed checked lowercase `EvmAddress` and `EvmHash`, canonical decimal
`EvmU256`, exact `CanonicalBytes`, and `NonZeroU64` chain IDs and gas limits. A transaction binding
fixes chain ID plus expected genesis hash, endpoint reference, authority epoch, and sender account.
The sole transaction command is nonce-free EIP-1559 type 2 with an empty access list and a private
bounded Create or Call action. Its complete factories and checked deserializer enforce input bounds,
priority-fee ordering and nonzero gas. Plans and commands retain one shared nested parameter
product. Its private fee amounts store `u128` and serialize as canonical decimal strings; factories
accept `u128` fees and checked decoding rejects noncanonical spelling and overflow. `CheckedCreatePlan` and
`CheckedCallPlan` share these validation owners; the latter has no target. `CheckedTargetCallPlan`
adds a required target for an ordinary call. Deserialization preserves all plan checks.

`EvmTransaction<C, R>` authors a transaction with one selected `TransactionRecipe<C>`.
`CreateAt`, `CallCreatedAt`, and `CallAt` select checked plans and, for creation-dependent calls,
a required successful creation address. The reservation State's deterministic prepare callback
constructs the complete command before nonce reservation or IO. Capability injection expands
`ExecuteEvmTransaction<C, R>` into `ReserveEvmNonce`, `PrepareEvmTransaction`, the designated
execution Effect, and `ProjectEvmTransactionOutcome`, all with the same initial context and recipe.
The first three have `Never` domain failure; only authenticated reversion reaches projection's
typed `EvmTransactionFailure<ExecutedContext<C, R>>`. The recipe's sealed `Created` or `Called`
mode controls both command checking and projection. A local mode mismatch is Internal.

Each stage replaces one named field while preserving siblings. `ReservedEvmTransaction` retains
the complete command and reservation; `PreparedTransactionFacts` retains those plus full
preparation evidence; `ExecutedTransactionFacts` retains those plus full settlement;
`CompletedTransactionFacts<Created/Called>` adds the checked address or target. No fact nests a
preceding workflow context. Fact constructors and decoders check command reference, nonce domain,
settlement nonce/hash/action, and projection agreement. Runtime and Journal establish EffectId
provenance; decoding arbitrary JSON does not establish that a transaction ran. The reserved and
prepared capability command descriptors remain separate from these report records. Pending
execution manufactures no completed facts. Signed wire never enters Program or Journal.

Executable identity hashes a canonical domain-separated descriptor containing implementation
version, stage, explicit recipe identity, ordered selected slots, and outcome mode. Exact value
schemas remain additional ABI association keys. Cold fold re-prepares retained commands from
exact snapshots and does not rerun completed interpretation. A completed transaction adds seven
frames after admission; Program v3, Journal wire, and the Runtime fold remain unchanged.

Terminal reporting uses existing checked execution facts and settlement outcomes; there is no
separate persisted outcome projection. Products own root failure policy. The fixture reports retain
checked original plans once and an executed evidence prefix containing reservation, preparation,
and settlement. Continuation presence must agree with authenticated success or reversion. Checked
root decoding reconstructs commands and checks exact evidence references, target and observation
linkage. The root failure reason is derived from that prefix, never independently supplied.

The broad EVM Read intent fixes only a nonzero chain ID, physical-route content ref, and one of six
balance subjects; that subject is the operation discriminator. Its returned sum contains checked
chain IDs, anchors, raw units, or token decimals bounded to 0 through 30. The context-preserving
anchored contract-call Read has separate intent and evidence types fixing a transaction-route
content ref, target, bounded calldata, and exact block anchor. Every broad and anchored evidence
outcome carries Runtime's exact qualified intent value ref, which the capability binder checks
before its typed subject/result relationship. Returned anchored evidence additionally contains the
same anchor and bounded return bytes. Rejected, safe-failure, and integrity-blocked evidence project
to a closed failure reason. `ObserveAt` combines a checked observation plan with a selected
completed call's target and receipt anchor. It rejects cross-field chain/route mismatches before
provider entry. `ReadAnchoredContractCall<C, R>` constructs the intent during prepare, and retains
the exact intent and all accepted evidence in `AnchoredObservationFacts`, including domain failure
evidence. ABI decoding and product reporting remain explicit product semantics. These domain contracts perform no provider, signing, nonce, or persistence IO; the
authority and live adapter remain separate downstream responsibilities.

`register_evm_transaction_adapters` registers three callbacks under the same binding; application
composition separately calls `register_evm_transaction_states::<C, R>` for the four exact State ABIs. Reservation captures only
binding, custody, and provider; preparation captures binding, custody, and signer; execution
captures binding, custody, and provider. Registration checks epoch, signing purpose, and sender.
Each adapter checks its command binding before IO. Reservation loads existing custody before
observing chain and pending nonce. Preparation reuses retained wire before signing and validates
the actual immutable winner returned by custody. Execution decodes and qualifies exact retained
wire and recovers its sender without a signer handle. Pure hashing, encoding, decoding, and recovery
run in immediately awaited blocking work; IO and custody handles stay outside those closures.

Execution checks receipt before submitting at most once per invocation. A matching submission
returns Pending; transport failure, dropped acknowledgement, malformed response, or hash mismatch
is Unavailable. Settlement requires a validated receipt and matching canonical block identity.
Journal alone retains settlement; cold completed histories need no provider or signer call. This
canonical-receipt policy is limited to the pinned non-reorging development fixture and is not
registered by production composition. Displaced old transactions stay unresolved; there is no
automatic renonce, replacement, or terminal conflict policy.

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
domain with an advisory lock in an explicitly READ COMMITTED, READ WRITE transaction, then uses a
separate statement snapshot to allocate `max(provider_pending, highest_local_reserved + 1)`.
An existing Effect reservation remains unchanged. External account activity can advance fresh
reservations without requiring predecessor settlement. `u64::MAX` means exhaustion and returns
Unavailable without insertion; the largest reservable transaction nonce is `u64::MAX - 1`.
Reservations and prepared raw transactions are immutable facts in the v2 baseline. One
marker-driven physical-table load returns a reservation and optional prepared bytes. Prepared
custody returns the first retained winner even when a concurrent valid signature differs; the live
adapter qualifies that winner. Every write forces synchronous COMMIT. Ambiguous acknowledgement
is Unavailable and a later load resolves the committed-or-absent outcome. Old schemas are rejected,
not migrated. Writable rollback of an epoch is unsupported.

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
comparing one immutable revision. New admission requires an exact retained name and `sha256-jcs-v1`
digest and checks every requested binding before appending genesis. Start first reads the requested
RunId: a matching retained source identity resumes without requiring configuration custody; a
conflicting selection is rejected. Uncertain reads stop admission. The result includes the selected
config summary and run view. The management surface exposes import, complete unpaginated listing, and
idempotent exact delete. Deletion does not revoke runs already admitted from that revision. The shared
surface also owns compiled component, entry-point, and binding discovery, run progress/read, and
mechanical run-head listing. Compiled component discovery is an unexpanded inventory of the public
entry points and reusable Operations admitted by Application composition plus the exact States that
the same composition registers with Runtime.

Candidate enrichment uses ordinary anchored balance Reads and a final Pure selection State. It
retains every native source and tokens with nonzero raw balance, preserving candidate order,
configuration quotes, and route identities. Each collection requires a native source. Provider
failure fails the run; it does not remove candidates. Publication explicitly reads a successful
exact-schema enrichment output, resolves its bindings against the immutable composition inventory,
and imports an immutable snapshot config. It never executes discovery or admits another run.

New dependent admission verifies the enrichment RunId, terminal head, output ref, resolved values,
and bindings against that retained successful run. C0 retains complete resolved demand and the
bounded linkage, plus the selected source revision name, entry point, and 64 lowercase hex digits
of its canonical-document SHA-256 digest. This fixed revision field does not change the raw
ContentDigest grammar. RunView exposes its already-qualified genesis input and Program entry point
for matching admission recovery. Configuration deletion is not revocation; read/progress never
reload configuration or enrichment history. Re-enrichment is an explicit new run and revision.
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
