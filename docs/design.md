# Design

MFM durably proves caller-driven execution of an immutable typed Program. Program contains an
ordered sequence of State declarations with exact input, output and original failure contracts,
selected recovery policies and parameters, explicit root failure maps, scoped checkpoint positions,
and finite recovery allowances. Success advances linearly; recovery is a Runtime transition.
`Never` remains the uninhabited failure contract.

Source code authors the sequence through a deterministic `Operation`. Its required `validate_input`
check establishes agreement between root planning assumptions and the initial value before expansion.
`expand_program(entry, operation, input, limits)` qualifies that input and commits its exact value
reference into Program v8. Runtime rejects input substitution before genesis or provider entry;
cold reconstruction checks genesis against the same commitment. Parent planning and deterministic
States establish future child input agreement. No authoring callback enters Runtime. Multiple RunIds
may reuse the same exact Program/input pair.

OperationExpansion lowers Pure/Read/Effect States and child Operations through one private symbolic
draft. Scoped checkpoint tokens cannot be captured for direct installation in another scope, while
inherited installed bindings retain their owning scope for final relocation. Operations, callbacks,
and injection setup remain authoring-only. Program retains one selected handler implementation,
its exact parameter contract and qualified immutable parameters, checked target list, and allowances.
Selection is occurrence override, nearest explicit enclosing Operation, then framework Stop.
Replacement changes parameters and targets together; allowances inherit independently, including
explicit zero. Program v8 rejects superseded descriptors.

Changes to intrinsic classification semantics require revision of the exact error contract identity.
Errors implement the pure `ClassifyError` projection into `Retryable`, `OutcomeUnknown`,
`InputInvalidated` or `Permanent`. Typed Runtime runners classify each retained original cause directly. One static handler receives
that `Classification` and the derived `RecoveryContext`; original causes, complete executed inputs
and exact requests remain separately retained. There is no classifier registry,
policy-facing mapped incident, or classifier veto. Runtime alone authorizes the handler request.
`StandardRecovery` retries Retryable Reads/pending Effects and restarts invalidated Pure/Read input
only with exactly one declared target that is eligible; all other combinations stop. Custom handlers
may request retry of an unknown pending outcome under retained-command policy, without establishing
nonacceptance or authorizing a replacement command.

EVM provider errors retain one owner category and one reviewed provider source. The source owns
method, stage, local checked facts and Values-owned diagnostic data. Response status, RPC code,
message and original RPC data text are observations separate from ordered exposed source ancestry.
Diagnostic text uses the `diagnostic_float_free` persisted profile: ordinary numeric, structural
and actual-size limits apply, while dependency-supplied text bypasses the generic secret-marker
check. Ordinary Program/context strings retain that check. The derived FailureReport v5 uses the
same diagnostic profile for its already admitted Objects. No independent diagnostic quota applies. Runtime commits this exact original before classification or policy and retains it through cold
observation. Remaining upstream first-loss gaps are tracked in the owner audit inventory.

The framework and shipping Portfolio default to Stop with zero global/local allowances. EVM's
Read operational causes are Retryable; `AnchorChanged` is InputInvalidated. Transaction provider and
authority failures are OutcomeUnknown; signer unavailability before prepared-wire retention is
Retryable. Authenticated transaction reversion is a Permanent domain settlement failure.
`CollectEvmBalances` inherits its caller's selected handler.

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

Pure evaluation and Read/Effect interpretation return a proposed domain outcome or a reviewed
Values-owned `InvocationDiagnostic`. Preparation, typed decoding, handlers and maps forward the same
immutable data. Runtime retains the actual operation/stage in `InvocationFailure`; internal errors
never become persisted Program values or fault records. Pure and Read internal failures preserve
the current head. Effect interpretation runs only after accepted settlement is committed, so its
failure preserves `AwaitingInterpretation` and does not repeat external reconciliation. Owner field
conversion happens once; its two fixed failure markers retain the supplied code/operation/primary
size. There is no native error custody, reporting quota, source downcast or receiver recapture.
Only authenticated transaction reversion produces a reversion outcome; local action mismatches
remain internal execution errors.

Each Read or Effect occurrence applies the exact capability/State pair's authoring-time injection
policy. Before and after hooks use typed `OperationExpansion` scopes with the same linear State,
child Operation, handler and checkpoint authoring as Operations. The kernel inserts the
designated occurrence once between them. Hooks perform no IO or adapter registration and cannot
modify the designated occurrence. Nested hooks share a 16-level callback bound, counting the root as one, and the declaration
bounds. Before/after hooks are sibling levels. Nested entry is checked before descriptor
construction; suspended occurrence descriptors remain boxed during prefix authoring. Failed
expansions leave the parent draft unchanged. Trusted callbacks compose through OperationExpansion;
this is not a sandbox for unrestricted Rust recursion or stack allocation.
Empty hooks require identical input/output contracts. The complete expansion owns its input,
output, expanded failure contract and explicit designated-to-expanded failure map. Original failures
skip the after hook, which is a success continuation. Every suffix is checked before merging.

Runtime admits `(RunId, Program, C0)` and persists one complete `RunRecord`: `program_ref`,
`checkpoints`, `usage`, `effect_barrier`, and `operation`. `RecordedOperation` retains admission,
State success/failure, Effect preparation/settlement, or recovery. Its operation is the sole source
of the current input, cursor and phase. A private borrowed selector computes the permitted next
work for dispatch, validation and public observation; it is neither stored nor cached. A zero-State
Program succeeds in admission. Other Programs start at State 0/visit 0; a success selects the next
State with its output and a fresh visit, or becomes terminal success at the final declaration.

Every declared domain or operational failure is committed as `Failed(Failure)` before
classification, handler invocation, authorization, or root mapping. `Failure` owns its Pure/Read/
Effect operation and original once. A separate `Recovered` operation retains that failure,
classification, request and `RecoveryOutcome`. Retry/restart spend the applicable allowance once
and yield. Restart restores the selected checkpoint at a fresh visit and prunes later checkpoint
inputs without resetting accumulated usage. Stop stores a mapped root only for domain failures;
Read and pending-Effect Stop forbid a root. Terminal reports derive from that failure and outcome.
A policy, mapping or recording failure leaves the committed original authoritative. Restoration
selects authorized work without repeating classification, mapping or charging a grant.

Current validation checks operation contracts, positions, recovery authorization, sorted active
checkpoints and Effect identity/barriers. It does not compare a second phase or scan historical
transitions. Typed operation entry owns native input/intent/command/evidence consistency; a locally
valid substituted input is not authenticated by historical agreement. Public `FailureReport` owns
`Failure`, reason, usage and optional root, with borrowing typed accessors and its bounded canonical
artifact. Pending views own the unchanged `EffectCall` and optional original/outcome pair.
`RecoveryStopped` owns only its observed view; serializers borrow the pending facts for its existing
wire fields and status.

Effect execution commits the complete input, command and derived EffectId before adapter entry.
Pending returns the unchanged view without a record. Operational failure first commits the original
while retaining that authority, then a separate recovery decision returns to EffectPending. Retry
spends allowance and yields; Stop ends the invocation with `RecoveryStopped`, but explicit resume
may reconcile the same command. Accepted evidence is bound and committed as `EffectSettled` before
interpretation. `AwaitingInterpretation` resumes deterministic interpretation without adapter IO.
Settled Effects cannot retry or restart, and retained Effect barriers prevent restart across their
position. There is no pending-failure quota or future-capacity reservation.

Journal owns only the canonical `mfm.run.frame.v6` envelope: RunId, sequence, previous head and
opaque canonical payload. Its recursive head is SHA-256 over exact frame bytes. Runtime owns the
payload and current-operation relationships; Values owns each Object's exact value ref and canonical bytes.
Objects carry neither duplicated contract refs nor native caches. Public contract refs are derived
when needed. Frames have no object table, back-reference resolver, or Journal lifecycle sum.
Values implements checked `Object::Deserialize`; Runtime derives decoding on its current payload
and requires complete input consumption. Object decoding rejects invalid checked identities,
canonical bytes, hashes and bounds before returning an Object. Enclosing records reject unknown
or duplicate fields and unknown variants, while accepting ordinary Serde structural forms,
including supported sequence forms. No record re-encoding comparison or decoder seeds are needed.
Stored-data rejection reports restore/decode with parser category, available line/column and the
rejection reason. Nested constructor ancestry, structured expected/actual identities and size
facts are outside this decoding contract; oversized nested Objects remain internal errors (500),
not typed size failures (422). Subsequent slot admission and direct typed construction keep their
structured identity/size diagnostics. Declared execution originals and selected native constructor
hooks retain their separate causal contracts.

Store loads one mechanical snapshot containing the head, admission row, latest row, and optionally
one requested candidate-sequence row. Identical selected rows share ownership. PostgreSQL uses one
repeatable-read transaction and bounded row selection; memory Store takes one lock-protected
snapshot. Runtime binds selected headers and Objects, associates the admitted Program, and checks
current operation, contracts, positions, usage limits, checkpoints and Effect authority.
It does not replay or prove historical counter increments from earlier frames. Immutable acknowledged
history and Store's atomic exact-head append remain required trust contracts.

Known insertion adopts the candidate continuation locally. This attempt's `NotInserted` result
performs one exact candidate probe: presence can adopt a checked current observation; exclusion or
absence remains a noninsertion failure. Candidate presence is independent of later payload/view
projection, and any reload failure remains secondary. These paths yield without executing another
visit. Store errors return immediately without a probe. Indeterminate never becomes a claim of
insertion or noninsertion. BeforeAppend requires an admitted Failure and retains its separate
concrete Runtime cause without unsent candidate bytes. Store and NotInserted retain the admitted
original when present, independent causes and exact submitted bytes. Failed first-original encoding
reports known position/contract and unavailable original detail/identity, without retry or append.
Public reports expose candidate identity,
not its complete frame. Known insertion followed by projection failure retains the acknowledged
RunSummary separately from any older last-observed view.

No operational result is acknowledged before its original is durably inserted. Cancellation or
crash between provider IO and append may leave an unrecorded physical attempt. The audit guarantee
covers acknowledged originals; there is no fallback record through the failed Store, background
finalizer, or claim that an interrupted physical attempt was recorded.

The fixed limits are 32 MiB per canonical run object, 65,536 non-payload envelope bytes, 134,283,264
bytes per frame, 65,536 frames, and 512 MiB of frame bytes per run. Values enforces actual object
bounds, Runtime checks actual non-payload metadata, Journal checks complete frames, and Store checks
actual accumulated count and bytes with checked arithmetic. Recovery allowances govern semantic
decisions only. Actual results can exceed these limits after IO; rejection preserves the
acknowledged continuation and any unresolved command authority.

Hot advancement and cold restore use the same current-state checks. Native materialization occurs
at the selected typed callback. Before reconciling an unresolved Effect, Runtime deterministically
re-prepares and compares its exact command and identity. It does not rerun completed interpretations,
classifiers, handlers or maps merely to read a run. Public RunView derives from admission, current
commit and head; it does not expose the private checkpoint/usage representation wholesale.

Signing owns the checked transient recoverable-secp256k1 public key, 32-byte digest, low-S compact
recoverable signature, public recovery, and key- and purpose-bound `Secp256k1Signer` contract.
The in-process keystore constructs its non-`Send`, non-`Sync` key map inside one dedicated owner
thread, accepts only checked zeroizing scalars, retains at most 64 distinct key instances, and
communicates over a bounded channel. Purpose is immutable on each returned handle and does not cross
the owner channel. Duplicate same-key imports return handles with the same public key and may
carry different purposes without consuming another key slot. Explicit async shutdown requests exit
and immediately awaits an OS-thread join in blocking work; dropping the final sender also ends the
owner loop.

Portfolio admission retains checked `ConfigName` and `DigestBytes` identities. Their named value
grammars delegate to IDs; raw strings are checked at ingress, not re-parsed by Application.

Inline failure reports retain original and mapped payloads even when identical. Each report is
limited to 32 MiB independently of its individually qualified cause values. Admission reserves
neither future frames nor combined reports. Report overflow returns `size_limit_exceeded`
before the terminal append, preserving the acknowledged AwaitingRecovery head and its original facts and any
pending command authority; repeated explicit progress can encounter the same limit. No report is
persisted separately. Size errors identify the resource and limit without payload contents, distinguishing actual
measurements from lower bounds when serialization stopped before visiting the suffix; unrepresentable capacity arithmetic is a distinct invocation error.

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
settlement nonce/hash/action, and projection agreement. Runtime binds EffectId provenance in its opaque Journal payload; decoding arbitrary JSON does not establish that a transaction ran. The reserved and
prepared capability command descriptors remain separate from these report records. Pending
execution manufactures no completed facts. Signed wire never enters Program or Journal.

Executable identity hashes a canonical domain-separated descriptor containing implementation
version, stage, explicit recipe identity, ordered selected slots, and outcome mode. Exact value
schemas remain additional ABI association keys. Cold reconciliation re-prepares unresolved commands
from exact inputs without rerunning completed interpretation. A successful three-Effect transaction
and final Pure projection add ten frames after admission under Program v8 and Journal frame v6: each
Effect prepares, commits settlement, and commits interpretation separately.

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
Runtime retains settlement in the opaque Journal payload before interpretation; cold completed
runs need no provider or signer call. This
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


Loads use one read-only repeatable snapshot of admission/latest and any requested probe row. Appends take the per-RunId advisory transaction lock
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

Enrichment output pairs every resolved collection configuration with its route and observed anchor.
Portfolio identity and selected quote are stored once; consuming projection produces the existing
snapshot configuration and selector. Publication retains the exact output ref in provenance: a
changed output identity changes the configuration revision, while repeated publication of the
same qualified terminal output is idempotent. Superseded output schemas are rejected; histories
and existing revisions are never rewritten.
