# Design

MFM durably proves caller-driven execution of a complete immutable typed Program. Program contains
an ordered sequence of State declarations with exact input, output and original failure contracts,
selected native implementations and public bindings, typed callbacks, recovery policies, scoped
checkpoint positions and finite allowances. Success advances linearly; recovery is a Runtime
transition. `Never` remains the uninhabited failure contract. No root failure map is retained.

Sealed typed sources compose `Pure`, `Read`, `Effect`, tuples, endomorphic vectors, checkpoints and
maintained `Operation<Definition, Defaults>` scopes. `Plan` borrows a checked local configuration;
it never simulates future State input. `From<Definition>` supports explicit definitions with the
same maintained defaults as default construction. `compile(entry, source, input, resources, limits)`
discovers installed support once, then lowers a declaration-only draft while checking selected Rust
owners against that immutable inventory. A conflicting or uninstalled selected owner is rejected;
compilation never substitutes installed code or installs the selected source implicitly. The draft
commits the input value reference into Program v9 and calls Inventory association.
`load(bytes, resources)` decodes the document and calls that same association without Plan, source
configuration, resolution or injection. Association qualifies available public bindings and typed
handler parameters for every occurrence before binding any native resource. Each bound handler
captures its decoded parameters once; invocation retains only execution/error/panic responsibility.
Both paths return the same complete non-generic Program. Runtime rejects input substitution before admission or provider entry.
Multiple RunIds may reuse the same exact Program/input pair.

One nesting guard rejects excessive depth before Plan or native injection. Checkpoint marker types
resolve to unique qualified positions; construction failures retain reviewed identities, positions
and rejection reasons. Maintained defaults select handler parameters and checkpoint targets as one
unit; allowances inherit independently, including explicit zero. The default is Stop with zero
allowances. Complete document serialization, including aggregate bindings, uses the bounded encoder;
cold byte limits are checked before parsing/copying. Program v9 rejects superseded descriptors.

Changes to intrinsic classification semantics require revision of the exact error contract identity.
Errors implement the pure `ClassifyError` projection into `Retryable`, `OutcomeUnknown`,
`InputInvalidated` or `Permanent`. Program-owned typed callbacks classify each retained original cause directly. One static handler receives
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
check. Ordinary Program/context strings retain that check. The derived FailureReport v6 uses the
same diagnostic profile for its already admitted Objects. No independent diagnostic quota applies. Runtime commits this exact original before classification or policy and retains it through cold
observation. Remaining upstream first-loss gaps are tracked in the owner audit inventory.

Run Store's Unavailable, CorruptPhysicalState and Indeterminate each carry mandatory
DiagnosticEvidence in their existing externally tagged snake_case wire. Runtime retains the
concrete Store error and App/clients forward its data without recapture; no Store failure becomes
a durable operational original. Size/arithmetic variants and acknowledgement semantics are unchanged.
Selected PostgreSQL run calls retain operation/stage, SQLx layers and reviewed database/IO fields;
failed precommit writes keep an observed rollback error separate from the primary sources.
Selected SQLx dependency text follows the same diagnostic trust boundary. Local physical checks,
allocation and task failures retain their facts without rejected rows, queries, arguments,
connection objects or panic payloads. Configuration, gates and index errors remain separately scoped.

`source_cycle: true` means exactly “traversal stopped on a repeated interface pointer.” The local
source walker compares complete `dyn Error` pointers with `std::ptr::eq`; it does not establish
concrete-object identity. An inline child may share its parent's data address, and one concrete
error can have different interface representations. The latter may produce repeated cause entries
before termination; no exact cyclic-object visit count is promised across compiler configurations.
There is no identity registry, message comparison or diagnostic budget.


Transaction operational error v4 retains required diagnostic evidence on AuthorityUnavailable and
SignerUnavailable. The provider variant and typed classifications are unchanged. Authority's port
returns either unavailable evidence or the final internal invocation diagnostic; Live moves these
through the existing operational/invariant routes. PostgreSQL owns extraction of selected execution
SQL and retained-fact causes. Executing keystore failures carry SignFailed evidence directly through
the owner reply and into SignerUnavailable. Existing signer Invalid/Failed values retain their unit
kind honestly; checked primitive constructors remain outside this enrichment.

The current error changes the content-addressed native capability/State ABIs of Programs using it.
Cold association rejects superseded contracts; no legacy decoder or history rewrite is provided.
Journal and unaffected value schemas retain their current identities. Deployment data handling is
outside this source cutover and must not be confused with verification of new executions.

The framework and shipping Portfolio default to Stop with zero global/local allowances. EVM's
Read operational causes are Retryable; `AnchorChanged` is InputInvalidated. Transaction provider and
authority failures are OutcomeUnknown; signer unavailability before prepared-wire retention is
Retryable. Authenticated transaction reversion is a Permanent domain settlement failure.
Shared collection operations inherit their caller's selected handler.

Pure States deterministically map typed input to typed success/failure. Read States deterministically
prepare typed intent, then interpret typed evidence. Effect States deterministically prepare a
complete command and interpret typed settlement evidence. Program construction associates all values, mode-specific State executables, recovery callbacks
and exact native adapters before returning the executable Program. A semantic type ID names a value family and may have multiple exact
generic schemas; only the exact content ref selects a codec. State association likewise uses the
exact implementation/input/output/failure ABI, so one implementation identity may own multiple
generic ABIs without ambiguity. Read callbacks and evidence binding receive the exact intent value
ref; Effect callbacks receive the exact command value ref used for `EffectId` derivation. These are
instance refs, never shared codec contract refs. State code has no ambient IO.

Values owns `Unsigned256`, the checked canonical decimal integer in `0..=2^256-1`, including
checked addition. EVM's `EvmU256` delegates grammar/range checking to that owner while preserving
its native v1 schema and decimal wire. Shared and native scalars remain nominally distinct.
Chain's shared `BalanceRequest` admits 1 through 64 declaration-ordered sources with unique public
IDs and one exact ledger envelope. Source IDs retain the existing nonempty, 256-byte, control and
secret-marker checks; `DecimalScale` admits 0 through 30. Typed constructors and stored decoding use
the same invariant checks. `BalanceRead` binds every outcome to its exact intent and native original,
and an Observed outcome to the requested point. Unsuccessful authenticated outcomes remain explicit
Rejected, SafeFailure or IntegrityBlocked evidence with no invented amount. Native implementations
own protocol qualification. Shared BalanceContext, PreparedBalance and CandidateBalance retain exact
typed continuation and derive the active source from completed prefix length. ObserveBalance creates
only candidates or declared Permanent observation failures. Candidate construction/decoding performs
no scaling or completion. Native confirmation must compare anchors before shared append, whose nested
result separates local invocation errors from arithmetic rejection. Completed-prefix decoding checks
order, ledger, common point and individual scaling, allowing total overflow to remain a consolidation
failure. Protocol observation and acknowledgement are exercised through explicit native adapters;
verification scope and managed acceptance are recorded in [the Phase B ledger](dsl-phase-b.md).
ConsolidateBalanceCollection rejects an incomplete context with a local diagnostic retaining expected
and completed counts. Aggregate overflow is a declared BalanceCollectionFailure after all sources
confirmed. BalanceCollectionCompletion retains that context and the canonical total; decoding
requires completeness and recomputes the total, rejecting forged totals or overflowing stored
completions. Native confirmation grants no aggregate-success guarantee. The caller consumes the
checked completion to resume its own typed continuation and construct its product output.
Native client admission returns checked semantic Portfolio input rather than a resource-free
Program. Maintained snapshot/enrichment Operations derive collection/source vectors from that input.
Shared `BalanceSourceDefinition` carries source, ordinal, scale and an opaque native execution
descriptor alongside the independent expected route. Native configuration decoding and route
qualification belong to the native implementation/client. Portfolio retains semantic collection
metadata and requires request/descriptor count and route agreement. Enrichment pairs each required
source with its descriptor, validates membership/coverage, and keeps required or nonzero observed
sources. Native clients choose native-specific required sources and construct publication wire from
retained public descriptors. Portfolio contains no EVM planner, decoder or renderer.
Chain's private eighty-digit decimal arithmetic preserves full-width raw U256 inputs, canonical zero,
dust-to-zero and exact-remainder scaling. Capacity failures retain the actual digit count when rejected
and whether scaling or summation failed. Failure decoding checks the eighty-digit limit, possible
operation bounds and the supplied facts of an inexact scale. Values' `SizeLimitExceeded` now has a
checked typed-value contract; decoding requires actual greater than limit. Collection consumers still
require migration from the previous EVM arithmetic before these guarantees describe integrated runs.
EVM domain construction error v3 retains `Unsigned256Error` and checked `DecimalScaleError` as concrete
sources and persisted causes rather than collapsing construction rejection into `InvalidValue`. No rejected input text is
retained. The nested provider/transaction operational-error schema hashes therefore change, and exact
association rejects their former ABIs; their classification semantics stay unchanged. Transaction
commands, native scalar identities and evidence retain their prior contracts.

Native balance intent and subject v2 retain one account/asset target and stage-only subjects. The
intent also retains source ordinal, collection scale, chain ID and route reference for selected
supporting-binding admission. Its external subject tagging replaces the old source-bearing wire and
custom decoder; old intent shapes are rejected. EvmBalanceLedger preserves chain-ID-only balance
qualification without a placeholder genesis hash. Supporting identity native implementations check
all retained qualification facts against EvmBalanceBinding and preserve original EvmReadEvidence.
Supporting native States check chain identity, initial anchor, optional token decimals and final
anchor confirmation while retaining the shared typed continuation. Every source agrees with the
first completed observation point. Final confirmation uses the retained full-width block number;
an authenticated changed anchor is InputInvalidated before arithmetic admission. Authenticated
rejection, safe failure, wrong chain and integrity block are Permanent, with distinguishable
failure payloads. Local evidence identity/type mismatches remain invocation errors. Changed-anchor
failure decoding rejects equal anchors; arithmetic failure retains its concrete shared source.
Native boundary tests cover State qualification; production cold execution and failure projection
through actual adapters and Runtime are recorded in docs/dsl-phase-b.md.
EvmNativeBalance and EvmTokenBalance translate the designated shared balance intent and project all
four native outcome variants without re-encoding the original. They reject mismatched ledger,
account/asset and native evidence identity/type. Their fixed prefixes contain two and three Reads
respectively; both inject one confirmation Read as suffix. Focused resolved-source construction and
cold-loading tests cover nine Reads followed by shared consolidation. Application selection, both
continuations and actual cold Runtime execution pass production acceptance. ReadBalanceAt v2
retains the collection's public route reference.
Native translation checks it against the designated binding before IO. A cold Program with an
alternate designated route is rejected at invocation, even when its complete document and initial
value are otherwise admissible. This preserves the distinction between Program content identity
and agreement with the caller's retained route.

The shared Chain `CheckedAdd` Pure State consumes `CheckedAddition` and returns `Unsigned256`
or the exact Permanent `AdditionOverflow` original. Both operands remain in the complete State
input; the empty overflow payload does not duplicate them. Input constructors retain the rejected
operand and concrete scalar cause. Program's checked implementation-identity conversion retains
its originating checked-string grammar/reason through State implementation-ref derivation.

Shared contract lifecycle contexts require Applied deployment/configuration evidence and consistent
ledger envelopes. Observed context binds to the semantic intent reconstructed from the deployed
locator and configuration settlement's exact point. It may contain a scalar mismatch, which Validate
classifies as its Permanent domain failure; validated context and the flattened report require
equality with effective configuration. The report retains request, effective, deployment,
configuration and observation once, and its decoder reuses the same borrowed checks. These checks
do not authenticate native evidence, reconstruct unavailable prepared commands, equate request-specific
implementation ABIs, or constrain effective configuration to one particular arithmetic composition.

Exact native implementation references use the complete v2 descriptor: selected family identity,
semantic capability (including mode), semantic request/evidence, native request/evidence, operational
error and binding contracts. Program's typed reference helpers and constructor share this derivation.
The configured family StableId remains a selection key, not an exact implementation ABI reference.
Supporting preparation derives the designated ABI from installed types and binding identity from
the retained command; it does not add a configuration lookup or occurrence-specific identity cache.

Contexts are ordinary immutable `MfmValue` snapshots. Maintained shared predecessor and result
contracts own preservation of earlier facts; native recipes consume those typed requests. No
caller-defined slot projection, context service or Runtime codec registration is required.

Pure evaluation and Read/Effect interpretation return a proposed domain outcome or a reviewed
Values-owned `InvocationDiagnostic`. Preparation, typed decoding and handlers forward the same
immutable data. Runtime retains the actual operation/stage in `InvocationFailure`; internal errors
never become persisted Program values or fault records. Pure and Read internal failures preserve
the current head. Effect interpretation runs only after accepted settlement is committed, so its
failure preserves `AwaitingInterpretation` and does not repeat external reconciliation. Owner field
conversion happens once; its two fixed failure markers retain the supplied code/operation/primary
size. There is no native error custody, reporting quota, source downcast or receiver recapture.
Only authenticated transaction reversion produces a reversion outcome; local action mismatches
remain internal execution errors.

Runtime's `program_document` is a read-only bootstrap, not a RunView or authority token. One existing
Store admission/latest snapshot supplies both qualified Journal envelopes. The shared admission
parser checks the admission variant, Program Object/reference agreement and metadata bounds, then
returns that retained Object without re-encoding or executable association. It checks snapshot
RunId, sequence and head linkage through the ordinary frame qualifier. Program owns canonical
Program validation; bootstrap does not parse the latest Runtime record or validate continuation.
Malformed admission decoding retains parser category, location and rejection reason. All failures
carry the requested RunId and no fabricated last observation. Later read/resume obtains a fresh
snapshot; progress after extraction is allowed and extraction grants no append authority.

Program's kernel `callback` module owns typed State evaluation, preparation, evidence binding,
interpretation and original classification, plus bound adapter invocation, native decoding,
proposed-value/original encoding and panic containment. Outcome and adapter wrappers capture their
exact original contract at construction; Runtime supplies the position, retains the complete call
and command facts, and adds the operation. Runtime State runners are nongeneric and never pass
DriverContext or append authority into Program. Evidence binding and interpretation are separate
calls; an acknowledged Effect settlement remains authoritative when interpretation fails. Capabilities owns `CallbackFailure`, carrying Decode, Execute or
Encode together with the original immutable invocation diagnostic. Runtime attaches its actual
operation and forwards that diagnostic without recapture. A failed first original encoding retains
the known position, declared failure contract and encoding cause, with original detail/identity
explicitly unavailable; it never retries serialization. Ordinary encoding-job panics report Encode,
and adapter native-decode failures/panics report Decode. Panic payloads are excluded from returned
diagnostics; this does not control the process panic hook. IO remains in the async adapter future,
while pure decode, execution and encode jobs are immediately awaited. Outcome/adapter wrappers
do not classify an original or make a durable-recording claim. The separate classifier is invoked
only after Runtime acknowledges the original; its decode/execution failure leaves that acknowledged
original awaiting recovery. Capabilities owns `EffectAdapterOutcome` Pending/Settled.

Each Read or Effect selection resolves one installed native implementation. Its typed injection
returns a prefix and success suffix; the compiler inserts the designated State exactly once.
Supporting selections reuse the same compiler and binders. No hook performs IO, mutates the
selected occurrence, installs callbacks through Runtime, or maps originals into a root failure.
Empty hooks require equal endpoint contracts. Nested scopes share the single depth guard, checked
before planning/injection; siblings do not accumulate depth. Construction returns no partial Program.
This is not a sandbox for unrestricted Rust recursion or stack allocation.

Runtime's `execute` and `start` accept a complete Program and borrowed input. Their shared admission
encodes once synchronously with panic containment, an explicit exception to the blocking-work rule.
Encoding faults retain the requested RunId and Encode phase without inventing an observation. Exact
input contract and value commitment are checked before persisting one `RunRecord`. Execute continues
the existing driver to a checked terminal result; start returns at a manual progression boundary.
The current record requires domain `mfm.runtime-record.v1` and contains `program_ref`,
`checkpoints`, `usage`, `effect_barrier`, and `operation`. `RecordedOperation` retains admission,
State success/failure, Effect preparation/settlement, or recovery. Its operation is the sole source
of the current input, cursor and phase. A private borrowed selector computes the permitted next
work for dispatch, validation and public observation; it is neither stored nor cached. A zero-State
Program succeeds in admission. Other Programs start at State 0/visit 0; a success selects the next
State with its output and a fresh visit, or becomes terminal success at the final declaration.

Every declared domain or operational failure is committed as `Failed(Failure)` before
classification, handler invocation or authorization. `Failure` owns its Pure/Read/
Effect operation and original once. A separate `Recovered` operation retains that failure,
classification, request and `RecoveryOutcome`. Retry/restart spend the applicable allowance once
and yield. Restart restores the selected checkpoint at a fresh visit and prunes later checkpoint
inputs without resetting accumulated usage. Stop retains its reason without a mapped root.
Terminal reports derive from the complete original failure and outcome. A policy or recording
failure leaves the committed original authoritative. Restoration selects authorized work without
repeating classification or charging a grant.

Current validation checks operation contracts, positions, recovery authorization, sorted active
checkpoints and Effect identity/barriers. It does not compare a second phase or scan historical
transitions. Typed operation entry owns native input/intent/command/evidence consistency; a locally
valid substituted input is not authenticated by historical agreement. Public `FailureReport` owns
`Failure`, reason and usage, with borrowing typed accessors and its bounded canonical
artifact. The v6 report includes RunId, Program ref, State implementation ref, execution ABI and the
complete serialized Failure, including its input, intent or command, native evidence and original
where applicable. It is a diagnostic projection, not execution authority. Pending views own the unchanged `EffectCall` and optional original/outcome pair.
`RecoveryStopped` owns only its observed view; serializers borrow the pending facts for its existing
wire fields and status.

Effect execution validates selected native command extraction before committing the complete input,
semantic command and derived EffectId. Adapter entry follows acknowledgement.
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
Typed values may retain nested Objects using Values' structural `SchemaShape::object()` shape.
Its canonical payload uses the diagnostic float-free profile so generic custody preserves supplied
diagnostic text. This neither certifies secret absence nor permits deliberately adding MFM secrets.
Structural schema admission does not validate the nested content hash; checked typed decoding uses
`Object::Deserialize` for exact canonical bytes, identities, hashes and bounds. The native owner
must separately establish the selected nominal contract and any ledger or protocol relationships.

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
classifiers or handlers merely to read a run. Public RunView derives from admission, current
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

Inline failure reports retain the complete executed call and original without root mapping. Each report is
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
accept `u128` fees and checked decoding rejects noncanonical spelling and overflow. Native recipes
use `Eip1559Options` and the checked command factories for creation and existing-address calls;
there is no separate checked-plan wrapper.

EVM supporting States consume the complete typed semantic request. ReserveEvmNonce<R> constructs
its nonce-free command before reservation acknowledgement and retains ReservedRequest<R>, containing
only that request and ReservedEvmTransaction. Checked construction/decoding rejects request/command
mismatch. Reserved descriptor construction preserves canonical identity failure causes and distinguishes
command reference from nonce-domain mismatch.

The concrete scalar recipes qualify EvmScalarContractArtifact against the exact maintained
MfmEffectFixture.sol output from solc 0.8.33, Cancun, without optimizer, using the managed source
path. The artifact's SHA-256 identity binds its supported configure/value ABI; arbitrary bounded
initcode does not establish scalar-contract semantics. Compiler output stays temporary. Native
execution options reuse the existing nonzero gas and canonical u128 fee representation/order checks.
Deployment transfers zero value and uses the qualified creation bytes; configuration transfers zero
value and encodes the retained effective scalar into one full-width ABI word. Shared contexts never
decode calldata. Native binding resolution checks the selected family, artifact and exact ledger
before returning the public binding.

PrepareEvmTransaction<R> consumes the checked pair and retains a public PreparedEvmTransaction Object
inside shared PreparedTransaction<R>. Its binding reference derives from the retained command's
binding; its designated implementation reference comes from Program's exact typed descriptor helper.
No fresh configuration or runtime registry is consulted. Signed bytes remain in the authority.

The designated EVM implementation checks selected implementation/binding identity, the exact native
schema and request/command correspondence before execution. Settlement projection preserves existing
EffectId, nonce, transaction-hash and action checks, retains its exact original Object, and constructs
shared ledger/transaction/point envelopes. Reversion becomes Rejected with unavailable reason; shared
Deploy/Configure return their own semantic failures. Supporting reservation/preparation remain distinct
native protocols with identity translation. Preparation's authority-side command/wire checks remain
necessary: its public evidence alone carries only EffectId and hash.

Slot recipes, cumulative phase-fact wrappers, ExecuteEvmTransaction, the mandatory Pure outcome
projection and their wrapper Operation are removed. Shared lifecycle callers use the existing
compiler/capability/Runtime path. Intent, anchor, ABI and exact original-evidence checks remain native
responsibilities. [The verification record](dsl-phase-b.md) identifies the tested production revision
and managed evidence; supporting-State tests alone do not prove custody recovery.

The broad EVM Read intent fixes only a nonzero chain ID, physical-route content ref, and one of six
balance subjects; that subject is the operation discriminator. Its returned sum contains checked
chain IDs, anchors, raw units, or token decimals bounded to 0 through 30. The anchored contract-call Read has separate native intent and evidence types fixing a
transaction-route content ref, target, bounded calldata, and exact block anchor. Every native outcome
carries the exact qualified intent value ref. EvmContractReadImplementation binds ContractRead to
EvmTransactionRoute, resolved through LifecyclePlanning from retained native configuration. Observe
has identity prefix and suffix; no observation-plan State or cumulative slot context is introduced.
Native translation checks the complete chain instance and decodes exact locator/point contracts.
Projection checks reconstructed native intent, evidence intent identity, returned anchor and exactly
one 32-byte getter result. Full-width decimal decoding belongs to EVM and creates ConfigurationValue;
Rejected, SafeFailure and IntegrityBlocked preserve their corresponding semantic outcomes and exact
native original Object. The live provider remains responsible for matching its route before IO.

ContractExecutionConfig v2 retains the caller's expected observation_route_ref independently of
native configuration and selected bindings. Shared Observe copies it into ReadContractValue v2,
and shared observation/report consistency reconstructs that complete intent. One EVM route
qualification function checks the native route's content identity during configuration resolution
and before native Read invocation, including cold-loaded Programs. Same-ledger endpoint disagreement
is an invocation-only internal binding failure with expected/actual public references retained;
it is never authenticated IntegrityBlocked evidence. No provider or outcome append occurs on
rejection. Runtime's initial admission frame remains distinct from Read execution; cold retry leaves
that head unchanged and makes no append call. Missing route fields in old wires are rejected;
changed nested descriptors update dependent schema identities without a compatibility decoder.
The old ObserveAt/AnchoredObservationFacts machinery is removed; shared Observe uses these contracts
in both direct framework composition and the managed lifecycle.

`EvmResources<Sources>` implements the existing environment and native binding contracts. Checked
transaction resources qualify authority epoch, signing purpose and sender at construction. Native
injection discovers reservation and preparation States automatically. Reservation captures only
binding, custody and provider; preparation captures binding, custody and signer; execution captures
binding, custody and provider. No registration helper or second executable assembly remains.
Each adapter checks its command binding before IO. Reservation loads existing custody before
observing chain and pending nonce. Preparation reuses retained wire before signing and validates
the actual immutable winner returned by custody. Execution decodes and qualifies exact retained
wire and recovers its sender without a signer handle. Pure hashing, encoding, decoding, and recovery
run in immediately awaited blocking work; IO and custody handles stay outside those closures.

Execution first checks the receipt, then transaction-known status. It submits the exact retained
wire at most once only when absent. Pending awaits one private second inside the native adapter;
there is no Runtime deadline, background task or replacement command. Cancellation preserves the
retained command for later reconciliation. Receipt, lookup and submission provider errors retain
their actual owner errors and causal diagnostics unchanged. Settlement requires a validated receipt and matching canonical block identity.
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
endpoint_ref }`. Native construction and resource binding qualify the same content ref. Credentials and
client handles are process-local and never persisted. Wrong local route/chain is `Internal` before
provider entry; only authenticated external evidence may become `IntegrityBlocked`.

Live composition accepts an empty or checked bounded set of native resources. Multiple endpoints
may bind one ledger. `EvmResources` owns exact read/transaction binding and derives public binding
views from those same records without a duplicate cache. Public endpoint names and independently
qualified route references remain in native execution descriptors for configuration-deleted
publication; they are not derived from whichever binding happens to be selected.

Application owns thin transport-neutral coordination. A strict XDG/HOME- or override-selected
`deployment.toml` names environment resolvers for the runtime PostgreSQL locator and stable public
EVM bindings; it contains no locator values and has no product lifecycle. Composition resolves each
private locator once and constructs PostgreSQL, stock EVM clients and checked native resources.
The concrete backend supplies Store, RunIndex and configuration custody. Native clients admit native
configuration and render semantic outputs; Portfolio owns collection semantics and checked product
projections. Application calls compile/load and Runtime, with no native decoding, registration loop,
handwritten component table or duplicate binding cache. The EVM HTTP client has no ambient proxy,
redirect, referer propagation or automatic retry.

The configuration repository retains complete, individually bounded canonical documents tagged by
the exact entry point. The native client admits the document before atomically creating or
comparing one immutable revision. New admission requires an exact retained name and `sha256-jcs-v1`
digest and checks every requested binding before appending genesis. Start first reads the requested
RunId: a matching retained source identity resumes without requiring configuration custody; a
conflicting selection is rejected. Uncertain reads stop admission. The result includes the selected
config summary and run view. The management surface exposes import, complete unpaginated listing, and
idempotent exact delete. Deletion does not revoke runs already admitted from that revision. The shared
surface also owns compiled component, entry-point, and binding discovery, run progress/read, and
mechanical run-head listing. Compiled component discovery is an unexpanded inventory of the public
entry points and reusable Operations plus States derived from the environment's owning source
definitions. Inspection requires neither Plan nor live handles.

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

Enrichment output pairs every resolved collection configuration with its route and observed anchor.
Portfolio identity and selected quote are stored once; consuming projection produces the existing
snapshot configuration and selector. Publication retains the exact output ref in provenance: a
changed output identity changes the configuration revision, while repeated publication of the
same qualified terminal output is idempotent. Superseded output schemas are rejected; histories
and existing revisions are never rewritten.

ContractValueEvidence v2 retains its exact native original and a ContractValueOutcome. Only Observed
carries a point and scalar. Rejected, SafeFailure and IntegrityBlocked become corresponding declared
Permanent Observe failures; they cannot construct ObservedConfiguration or a successful report.

## Material uncertainties

none within the current implemented contracts; deferred product capabilities are listed in
[known gaps](known-gaps.md).
