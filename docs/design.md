# MFM Design Contract

Status: superseded implementation contract during the accepted single-ingress cutover

Normative target: [`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](../RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md)

Implementation decomposition:
[`IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](../IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md)

The accepted RFC is the platform-wide source of truth for the target design. Until the cutover
rewrites this document alongside its owning implementation commits, the text below describes the
implementation being replaced and is evidence/inventory only where it conflicts with the RFC.

The text below previously defined the runtime, history, storage, replay, and application design now
being replaced. Its persisted schemas and Rust types remain an exact inventory of the old bytes;
they do not constrain the incompatible target cutover.

## Material uncertainties

none

## Central contract

```text
authored declaration-ordered program
  -> pure expansion
  -> certified program closure
  -> append-only structured run history
  -> callback-free verified cursor
```

An operation contains only author-visible `State`, `Match`, and `FanOut` structure. Child operation
calls are authoring sugar removed during expansion. Only `State` has an executable occurrence.
Matches, fan-out groups, lanes, fragments, joins, and operation outcomes never receive an
occurrence identity or a history transition.

Declaration order is semantic. Typed lexical slots establish exact producer, nominal contract,
dominance, branch, child-fragment, and fan-out provenance. A root completes with exactly one
nominal `OperationOutcome::Success` or `OperationOutcome::Failure`. Outcome helpers construct
structural values; they are not instructions.

## Non-negotiable invariants

- Run history contains exactly five record families: `RunAdmitted`,
  `StateTransitionCommitted`, `ExternalAccessAuthorized`, `ExternalAccessObserved`, and
  `RunClosed`.
- Every append is atomic. Its assigned records and exact newly reachable content-addressed object
  closure become visible together or not at all.
- Every append binds one exact predecessor, run sequence, append request identity, candidate
  digest, record hashes, object references, writer store scope, and writer epoch.
- The store owns qualification, the sole reducer, and projection, and is authoritative for hostile
  persisted input.
  No mutable cursor row and no second reducer exist.
- `RunClosed` shares the append that first makes the root outcome derivable, including a
  zero-state `RunAdmitted + RunClosed` append. No record is legal after closure.
- Runtime owns the only production path that can request semantic run-history mutation through
  `RuntimeHistoryPort`. The store's private adapter owns the sole reducer and append authority.
  Runtime performs at most one semantic transition or one audited external-access operation per
  `drive_once` call. Callers never supply a trusted run digest for admission; the journal owns the
  one `derive_run_id` rule over the store scope, tenant scope, entry-point operation, and
  invocation identity. Qualification re-derives that identity from the admitted coordinates, so a
  history whose envelope and `RunAdmitted.run_id` agree with each other but disagree with their
  preimage is rejected.
- Authority-bearing integration traits (`RuntimeHistoryPort`, physical-binding verification,
  wallet authority, and deployment credential issuance) require a
  workspace-private marker. Downstream callers can consume the completed products but cannot
  implement a look-alike authority from the visible method set.
- Every external operation is authorized durably before possible entry. One committed
  authorization can mint exactly one affine invocation authority.
- Every normal invoker completion is frozen as pending observation material and committed before
  state settlement or a successful drive result. Observation-append retries never invoke again.
- State callbacks are deterministic and perform no ambient IO. Network, signer, filesystem, fact
  scan, and wallet-authority work is reachable only through registered Runtime invokers.
- Manifests, programs, objects, facts, configuration revisions, exports, and semantic identities
  use exact canonical, float-free content addressing.
- Secrets, bearer bytes, signatures, provider text, paths, endpoints, and private authority
  material never enter programs, histories, objects, facts, outputs, traces, exports, or reviewed
  errors.
- Replay and every read projection use the store reducer and perform no callback, provider, signer,
  wallet-authority, or other live semantic IO.

## Structured program

The authoring pipeline is pure and ordered:

```text
child substitution
  -> capability expansion
  -> policy wrapping
  -> failure completion
  -> normalization and certification
```

Expansion is finite, bounded, deterministic, call-site-local, and visible in the certified
program. Injected support states are ordinary leaves and do not recursively expand. Certification
checks declaration order, lexical dominance, exhaustive closed-sum matching, exact child input and
boundary substitution, nominal results, failure plans, fan-out restrictions, implementation
manifests, entry policy coverage, and one total root outcome.

Structured normalization and denormalization use explicit bounded work stacks. They enforce the
same path-depth, provenance-depth, component-definition, and JSON-node limits at construction,
certification, decoding, and replay; the JSON-node limit is an eightfold envelope over the
component-definition bound for structured protocol and schema nodes. Exceeding any bound is a
typed rejection and never a process-stack operation.

Canonical JSON values and retained history frames are bounded at 32 MiB. Plain canonical-JSON
ingress applies that document budget before parsing and
again to the canonicalized bytes. These ceilings remain separate from the smaller string,
portable-export, and provider-proof budgets: they admit the largest qualified structured-program
value while keeping every transport and retained payload bound explicit.
One openable run is additionally capped at 65,536 atomic batches, 1,048,576 retained objects, and
512 MiB of canonical committed-batch bytes. Store lifetime and run count remain unlimited. Raw
loaders preflight those dimensions before cloning or accumulating a prefix and report a distinct
capacity failure for otherwise valid evidence. The same ceilings apply at append; before an
external authorization, accounting reserves two maximum successor batches for its observation and
settlement so entering a provider cannot strand the required durable evidence. Prior-run selection
uses the request's remaining byte and fold budgets before loading a producer prefix, and one scan
may fold at most 65,536 producer batches.
Typed base64url ingress applies its generated character budget before decoding, so hostile wire
strings are rejected before the decoded allocation; trusted byte construction remains subject to
the enclosing canonical-value or retained-payload bound.
Native canonical values additionally enforce the generated UTF-8 string, object-key, array-item,
and object-entry limits at the native-wire validation seam before recursive traversal.

Only the canonical `CertifiedProgramDocument` is execution authority. Its content-addressed root
binds the authored program, expanded program, expansion profile and proof, policy proof, component
closure, and secret-free implementation manifest. Admission resolves the trusted entry profile
from the qualified registry; a caller cannot select weaker predicates. A persisted verifier
extracts the authored object, repeats certification, and requires the exact root and complete
document. Its cache is keyed by certified content identity, never by a nominal entry-point id.
The closure digest inserts each already-canonical component value as raw JSON within its enclosing
canonical preimage. This preserves the exact retained object bytes while avoiding a JSON number per
byte representation that could inflate a valid qualified program past the generated 32 MiB
canonical-document bound.

Registered authored programs may be replaced only through the support-envelope API of process
qualification before a registry is finalized. That replacement changes which authored candidate a
future certification may accept under an entry's frozen signature. It does not grant callers the
ability to substitute a different expanded program or schema set for an already admitted certified
root: on cache miss the store invokes the concrete `AdmissionVerificationRegistry::verify_root` and
consumes private-field `CertifiedProgram` data keyed by certified content identity.

### Failure algebra

Every state and fragment boundary declares:

```text
FailureContract = Never | Typed(RetainedValueContract)
```

`Never` is the one sealed kernel sentinel. It has no `MfmValue`, schema, slot, producer, value, or
handler. Every typed failure boundary owns exactly one certified failure plan. Default propagation
is an ordinary `Pure + Never` mapper selected by the exact source contract. Custom recovery is
ordinary structured control. Protected failure provenance is affine and cannot escape its exact
call-site handler.

### Match and fan-out

`Match` selects one exact canonical tag from an already-defined registered closed sum. Branch
choice is derived from the committed selector value; it is never authored into history.

`FanOut` is non-empty (head-plus-tail), collect-all, declaration ordered, and limited to `Pure` and
`Read` states. Match-arm and fan-out-lane products retain structural origin in `LexicalValueRef`
independently of payload bytes. Structural path ordering is ordinal-first; labels are diagnostic.
The certified profile bounds lanes, occurrences, declarations, branch depth, and fan-out depth.
The production portfolio uses depth two: portfolio network lanes contain EVM-owned read lanes.
An unresolved barrier stops further scanning, and a re-assertable Read keeps its own lane as the
minimum action rather than exposing a later one. Joins retain declaration order independent of
physical completion order.

## Run history and the three qualified layers

`mfm-journal` owns strict data and identities. `mfm-store` owns legality and continuation, split
across three layers with deliberately disjoint responsibilities.

**Qualification** turns hostile bytes into trusted evidence. It proves canonical syntax and typed
decode, batch bounds, run identity, sequence, predecessor, writer lineage, append request, candidate
digest, record hash, commit digest, object identity and type, directly named references, five-family
record decode, logical-key uniqueness, certified-root correspondence, complete certified component
closure, and admission material. Program trust comes from one concrete operation —
`AdmissionVerificationRegistry::verify_root` — memoized by exact operation and persisted root, never
by a nominal entry-point identity. Qualification makes no workflow decision, so a qualified history
may still be semantically illegal.

**Reduction** is the one deterministic rule. `reduce_event` advances a run from `Unadmitted` through
`Admitted`, walking the certified program in declaration order and resolving matches, fan-out,
failure routing, lexical bindings, and the cursor. It emits typed artifact intents and closed
obligations; it constructs no retained bytes and calls no verifier, backend, or registry. The
consuming typestate is a compile-time boundary, not a convention:

```text
reduce_event  -> PendingSemanticStep   (coordinate-free, no successor)
compare       -> ComparedReduction     (assertions matched)
discharge     -> FinalizedReduction    (the only type exposing a successor)
```

**Projection** applies the finalized result. Reduction retains only semantic record and artifact
drafts; the compiler is the sole constructor of journal payloads and materializer of history
objects. Comparison alone owns the private token that binds the pending primary-record handle, so
obligation code and unrelated store siblings cannot manufacture or bind a successor. Obligations
discharge in one closed scope (`RetainedOnly` or `RetainedAndCurrent`, retained always first, never
current alone), and only successful discharge can construct the private final stage. The coordinator
then consumes that stage to seal one affine `ValidatedRunAppend`; sealing moves the committed batch
instead of cloning it, and only an exact newly committed backend result releases the prepared
successor and fact capability. A backend receives that value and compares and applies it
mechanically — it never matches a record family, frontier, or access kind to choose behavior.

Reduction derives:

- initial and current lexical bindings;
- declaration-ordered actionable occurrences and active fan-out lanes;
- outstanding Read and Effect access;
- semantic and physical journal heads;
- possible-entry, integrity-blocked, and closed status; and
- the exact terminal root outcome.

Public-read and recorded-replay purpose projections retain only the
reducer-derived `RunEvidenceStatus`; the actionable frontier and its capability,
input, and state references remain internal to the reducer and Runtime adapter.
Typed operation results that cross the terminal public boundary are separate reviewed
`PublicOutputs` contracts: they project only redaction-safe product fields, while recovery
closures, provider attestations, signed envelopes, and other verification preimages remain
internal evidence.

The sole offline replay entry point consumes one complete callback-free closure: the root history,
every exact producer prefix, every dense publication route, and concrete trust. It verifies every
required producer head and recomputes each retained positive selection before returning an opaque
`OfflineVerifiedRun` containing only recorded status and bounded fixation metadata. No raw-history-
only path can mint offline evidence, and the complete verified cursor, objects, and bindings never
cross the store boundary.

Incremental mutation returns a successor produced by that same rule. Candidate authoring reduces an
intent, lets the compiler author bytes, requalifies those exact bytes, and reduces the recorded form
from the same predecessor — so candidate acceptance already proves the path future replay executes.
Reducing every complete prefix from raw persisted batches must produce an equivalent verified run.
Every Runtime drive loads a verified predecessor from one backend snapshot and compares its complete
current projection. The exact-head projection is the snapshot point for a non-mutating load; any
later append is handled by the append path's exact-head compare-and-append. No historical successor
or head-only cache answer can become store authority.

Production physical qualification binds an exact purpose tuple: access kind, semantic capability,
semantic adapter, qualified adapter implementation, stable resource lineage when applicable, and
an ordered immutable physical-target release history. Each release retains its full public
certificate, admitted routing policy, exact target identity, predecessor certificate, and, for an
Effect successor, the public lineage head that activated it. Retained verification accepts any
retained release while reducing recorded history, but newly proposed access must select the
current release. A refreshed Effect authorization must be a strict descendant of its exact prior
certificate at the reduced non-rollback head; supersession evidence proves that same old-to-new
edge. Reconstructing a process with a longer retained history therefore preserves old runs while
preventing new authorization against superseded releases.

Memory and PostgreSQL implement the same store contract. PostgreSQL writes real SQL transactions,
uses exact-head compare-and-append, and opens only after a deployment-supplied authoritative writer
fence qualifies the store scope and epoch. An unavailable or unqualified database fails closed;
there is no memory fallback.

Semantic open owns one non-cloneable physical snapshot `S0` until the complete audit finishes.
It keyset-pages the union of history, projection, and publication run keys, then qualifies and
drops one run at a time. Each run is preflighted before materialization and is limited to 65,536
batches, 1,048,576 retained objects, and 512 MiB of canonical history; exceeding those retention
limits is a capacity failure, not evidence corruption. For that run, the audit compares the exact
reduced projection and derived publications against pagewise routes from the same `S0`. It then
keyset-pages tenants and checks each dense route and head without accumulating the store's
publication lifetime. Total store history remains unbounded because no store-wide snapshot,
verified-run map, or publication vector is constructed. The backend revalidates and releases `S0`,
and authority is validated freshly once more before Runtime or any reader is constructed.

## Runtime access choke point

Runtime interprets the minimum declaration-ordered actionable path from the verified cursor. Its
private access bracket is:

```text
Prepared<K>
  -> committed ExternalAccessAuthorized
  -> Authorized<K>
  -> exactly one registered invoker entry
  -> one affine invocation result
  -> committed ExternalAccessObserved
  -> exact returned-value or safe-failure settlement callback
```

The closed completion algebra is:

```text
Read   = Returned | SafeFailure | IntegrityFault
Effect = Returned | SafeFailure | SupersededBeforeEntry | EntryUnknown | IntegrityFault
```

Only reviewed `Returned` and `SafeFailure` evidence reaches state settlement, through distinct
returned-value and safe-failure callbacks. Safe-failure settlement is disposition-typed and, under
success-only disposition, admits only a success proposal for every inhabited value—no `Failure`,
no `InvalidEvidence`, and no author-selected sample corpus. An ordinary typed failure follows its
certified handler and may close the run as failed. `SupersededBeforeEntry` is Effect-only and,
after purpose-limited physical-lineage verification, creates exactly the next attempt ordinal.
Every Effect capability additionally declares a sealed re-entry discipline, orthogonal to refresh
and certified beside it. `EntryOnce` states that a repeat is not absorbed; `EntryAbsorbing<MAX>`
states that the external system absorbs a repeat of the byte-identical committed request, and is
declarable only over a request that names its exact entry key. Declaring absorption asserts four
things the kernel cannot verify — that the external system absorbs a repeat, that the adapter
transmits the key, that absorption is retained long enough, and that `Returned` and `SafeFailure`
are functions of the external system's post-state rather than of one exchange. The last is the
sharpest: `RowsAffected(1)` violates it and `InsertOutcome { key, row }` satisfies it. All four are
certification obligations discharged by review, and naming `EntryAbsorbing` is where an author
takes them on.

The possible-entry frontier names its blocked `EffectEntrySubject` — occurrence identity, normalized
occurrence path, unresolved access attempt, and semantic capability contract. Every one of those
fields is already exposed by the access-audit projection, so carrying the subject past the barrier
is redaction-safe by construction.

`EntryUnknown` parks possible entry unless the capability declares `EntryAbsorbing` with budget
remaining, in which case the occurrence is re-assertable at the next ordinal. A crashed Effect
attempt under the same declaration is first *closed*: Runtime commits an `EntryUnknown` observation
against the dead attempt carrying one reserved kernel fault code, reaching no adapter and authoring
nothing. That closure asserts nothing about the external system — only that the invoker authority
for the attempt is lost — and it is what keeps the single-outstanding-attempt rule unchanged on the
Effect path. A re-assertion must carry byte-identical `occurrence_id`, `state_input_ref`, request
contract, and request digest, which the reducer verifies. The reserved code is the only thing
distinguishing a synthesized closure from an adapter-reported ambiguity downstream, and no domain
may reuse it. Committed `IntegrityFault` blocks. A Read authorization with
no observation is re-assertable at the next ordinal: a Read is defined as consuming no externally
meaningful state, so reissuing it is sound by construction. That makes the definition load-bearing
and a certification obligation rather than prose — the reducer cannot distinguish a crashed Read from
a live in-flight one, because the leaf is a pure function of history and liveness is per-process, so
two workers may invoke the same Read concurrently. This is the single-outstanding-attempt rule's
only relaxation and it is written as an access-kind-conditional branch; the Effect path keeps the
rule verbatim.
None of these statuses blocks an unrelated run id. Adapter unavailability may leave an attempt
uncommitted; it cannot be recorded as a semantic failure by settlement.

Runtime commits the exact callback proposal. It does not synthesize an output, fact, failure, or
terminal meaning. Stale-head settlement reloads only when the semantic head is unchanged.
Authorization ambiguity is resolved before invocation; observation ambiguity is resolved before
rebasing; neither path duplicates external work.

Every non-persisted Runtime fault is one contextual, redaction-safe value. It binds a closed fault
code and phase to the run id, last verified head when one exists, current occurrence when one
exists, and either the exact registry-issued process identity or the structured-store
lineage/epoch. Process identity contains semantic kind, semantic contract, and qualified
implementation contract internally. Callback proposals retain that origin through candidate
qualification. Callback, codec, contract, invalid-evidence, and rejected-candidate faults append
nothing and leave the prior authoritative prefix unchanged; only an invoker-returned committed
`IntegrityFault` can reduce to durable `BlockedIntegrity`. Public projections may expose the phase,
run, occurrence, semantic kind/contract, or store identity, but never the implementation contract
or lower-boundary diagnostic.

## Facts and prior-run reads

Facts are typed members of a committed state transition and share its atomic object closure.
Every non-empty successful transition atomically advances one dense publication sequence keyed by
store scope, writer epoch, and tenant, and retains an exact route to that transition. Same-run
consumers use lexical program provenance.

Deliberate prior-run selection is an ordinary authorized Read. Admission retains one strictly typed
source manifest over entry operation, optional exact certified-program identities, and fact
descriptors. The request binds that admitted manifest, the fixed selector, the sole
complete-through-authorization-frontier mode, the other-run tenant scope, five total scan bounds,
and the ordered queries. Exact capability, adapter, implementation, and public-certificate identity
distinguish this Read from every general adapter.
Fact scalar subjects use the fact owner's unsigned-native canonical profile: booleans,
strings, and unsigned integers are admitted, while signed-number producer variants are rejected
instead of silently round-tripping as unsigned values.

The registry builder privately installs that exact capability/adapter pair as the sole kernel
process baseline. It is retained for Runtime dispatch but excluded from entry support closures that
do not use fact selection. Application assembly cannot register or shadow it, and the baseline does
not relax rejection of any other unused process binding.

Authorization atomically records the current tenant fact frontier without advancing it. Only a
directly acknowledged new authorization mints the store's non-cloneable one-use scan permit;
idempotent content and acknowledgement recovery do not. The permit exposes only a complete bounded
scan through the captured frontier. The scanner follows every dense route, verifies each producer
with the same reducer, applies the admitted source manifest, excludes the consumer run, and
uses the fixed deterministic top-k evaluator. It cannot append history or obtain generic store
query authority.

A positive response retains the request digest, exact source references, canonical selected
subject/response/claim bytes, deterministic identities, and an authorization/frontier completeness
attestation.
Exhausted bounds and backend unavailability are typed safe failures; missing, malformed, or
substituted provenance is an integrity fault. Live and offline verified loads traverse every
retained selection barrier through its dense frontier, including pending or safe-failure attempts,
and recompute every retained positive response. Offline verification additionally requires the
supplied producer prefixes and publication routes to equal that complete traversal exactly, so
replay and projections never treat a merely well-shaped completeness claim as authoritative.

## Replay and public projections

`mfm-replay` owns the portable export format and offline verifier, and projects the reviewed public
run view, transition trace, access audit, and replay summary from the same qualified reduction.
Replay summary, transition trace, and access audit each have a distinct private typed wire owner;
there is no generic projection wrapper or contract switch. Transition decode re-derives the exact
assigned record hash. Audit decode binds status to the exact optional observation and re-derives an
included observation record hash. Pages are fixed to one journal head, and an observation committed
after that head is projected as absent even when it exists in the currently loaded suffix. Portable
export first authorizes every referenced prior-run source under the sealed export purpose and only
then serializes exact committed-batch envelopes,
source relationships, and semantic/physical fixation with the exact target key, database identity,
fence generation, release epoch, and current-incarnation reference with a closure reference; denied
or incomplete source closures emit zero bytes. Offline verification uses only bundle bytes and an
explicit trust snapshot against the store's read-only semantic verification entry. The program,
retained-release, and store
lineage verifiers used by that snapshot are workspace-sealed deployment authorities; an
ordinary consumer cannot substitute callbacks that accept forged fixations. Current portable
exports use one bounded newline-delimited `Batch | Seal` record stream with media type
`application/vnd.mfm.structured-run-export-stream.v4`. The complete stream has one content identity;
individual records have none. The terminal seal binds the exact closure, fixation, authenticated
principal, fixed `export` grant, and content-addressed policy-decision references. Legacy streams
and retired bytes are rejected.

Authorization discovery uses the requested kind's exact root cutoff and recursively advances each
producer only to the maximum transition head required by selected fact routes. A later audit-only
suffix therefore cannot add a semantic-export dependency. Decision references in the terminal seal
are opaque content-addressed policy evidence; offline consumers do not resolve them live, and the
explicit trust snapshot must bind the exact closure digest before reduction.
Portable replay invokes the store verifier once for that whole closure, then checks every declared
source's complete journal and semantic fixation against the source metadata minted by that same
verification.

Recorded replay is verification-only: it reduces the committed prefix and returns its bounded
summary. It never falls back to live callbacks or compares current history.

## Configuration history

Application configuration is a separate append-only stream keyed by:

```text
(store scope, tenant scope, entry operation, target)
```

Each revision binds a one-based sequence, predecessor, configured-value contract, exact canonical
value reference, value bytes, and append request identity. Deployment receives configuration
append authority. Normal application assembly receives resolve-only authority and admits the exact
resolved revision as immutable run material. Configuration revisions are not a sixth RunHistory
record family. The shared canonical append boundary rejects any serialized revision larger than
`MAX_CONFIGURATION_REVISION_BYTES` before dispatching to memory or PostgreSQL; the configured value
itself remains subject to its schema byte bound.

An append request identity binds the exact stream key, predecessor, configured-value contract, and
canonical value. An exact historical retry returns its retained revision even after later
successors. Reusing that identity with any different bound input is an append conflict; only a new
identity presented against a non-current predecessor is a stale-head request.

PostgreSQL retains an independent exact `configuration_heads` compare-and-append row for each
stream. That local head detects removal, rollback, or divergence while the database remains under
the admitted writer generation. It is not evidence against a coordinated rollback of both the
revision rows and their local head: production qualification must also bind the configuration
writer to the deployment's external non-rollback fence authority. If that authority or its exact
current generation cannot be proved, configuration append and resolution fail closed.

## Production operations

The current production registry contains exactly three structured programs. Assembly passes those
three identities as the complete expected set, and registry finalization rejects a missing, extra,
or duplicate identity. The private kernel fact-scanner process baseline is not a fourth program:

- `PortfolioSnapshotOperation`: builds declaration-ordered network lanes, embeds the EVM-owned
  balance collection fan-outs, and aggregates the exact ordered results.
- `EvmBalanceCollectionOperation`: checks route and chain identity, observes an anchor, reads
  native or token balances, confirms the anchor, and returns an ordered collection.
- `EvmSubmitTransactionOperation`: expands into explicit domain/intent derivation, wallet status,
  fresh pending-nonce observation, reservation, candidate construction and attestation,
  activation, exact broadcast, bounded receipt/finality observation, reconciliation, completion,
  and public projection states.

Every retry, poll, replacement, recovery, and failure mapping is visible structured control. The
generic kernel knows nothing about EVM, wallets, nonces, candidates, or transaction lifecycle.

## Wallet nonce authority

`mfm-evm` owns canonical wallet-domain requests, responses, keys, proofs, and the narrow port.
`mfm-storage-evm-postgres` owns the real SQL activation registry and nonce authority. It does not
interpret run history or perform EVM IO.

Qualified deployment infrastructure is the sole issuer of chain-instance and route-membership
authority. A chain declaration binds its chain-registry lineage, never-reused instance namespace,
chain id, genesis, and immutable fork anchor. Its registry attestation additionally binds the
declaration reference, issuance reference, and registry head at issuance. A route generation binds
that exact qualified instance plus its membership issuance; a complete routing catalog binds the
current registry head, full attestations, and every admitted generation. The provider qualifies
that catalog against its pinned lineage and exact current head into an opaque, non-serializable
deployment value. Application assembly requires that value, derives the admission routing policy
from it, and requires the live catalog, wallet activation, portfolio route, intent, and submission
request to carry the same full chain binding. Numeric chain-id equality alone grants no authority.
The activation issuance reference commits the logical provider identity and activation record.
Startup hydration recomputes that commitment before admitting an issued activation, so a restart
under the same provider identity can recover it while another provider identity cannot borrow it.

Production deployment construction is one affine provider bracket. `BeginDeploymentAssembly`
consumes the qualified catalog and fixes the exact activation, store incarnation/public head and
provider fence, key-specific signer generation/fence/direct-path exclusion, code-derived semantic
contracts, complete ordered physical-release histories, routing catalog, and private target
inventory. The provider issues one bounded lease plus fresh per-route target challenges. After the
private endpoints return the exact ordered proof closure, `FinishDeploymentAssembly` rechecks all
current provider state and consumes the lease atomically before releasing its committed finish
authorization. The application consumes that nonforgeable result, the provider-neutral completed
exchange, and the qualified signer to construct transport, signer, balance, and wallet bindings
privately. Direct transports, preassembled bindings, stale leases, and caller-substituted public
references cannot construct a deployment. Revocation atomically closes Begin and invalidates all
pending deployment-assembly leases before waiting for drain; Finish can linearize only before that
cutover. Drain and promotion account for both ordinary authority leases and nonexpired assembly
leases, so no partially assembled authority can escape a revoked or promoted provider.

Signer attestation is eligible for a Runtime Read only through a process-local
`QualifiedReadSigningProvider`. The raw signer and every transitive generation guard must declare
an immutable observational contract: qualification and signing consume no quota, approval token,
anti-replay state, billing credit, rate-limit capacity, or other externally meaningful semantic
state. Keystore qualification rejects ineligibility before guard or key access; the affine handoff
reproves the complete key identity, generation, fence, direct-path exclusion, exact binding, and
eligibility; live construction accepts only the resulting opaque bearer; and every signing call
fails closed if eligibility or binding changes. The keystore signer uses a token-gated existing-file
open and decrypt path that cannot initialize, append its persisted audit log, lock for mutation, or
save. The superseded audited getter is deleted; historical `v1` key-access audit records remain
authenticated persisted data with no current producer. This observational behavior
is provider implementation `mfm.signing.keystore.rfc6979.v2`: append-only physical release history
retains the prior `v1` certificate and adds a `v2` successor even when the durable key-generation
target is unchanged. This qualification is executable process-local
authority, not a persisted descriptor field. An ineligible signer is rejected, with no Effect
reclassification or alternate constructor.

`WalletNonceDomain` is a qualified physical chain instance plus sender. Permanent activation
binds the full registry attestation, initial route generation and membership issuance, sender,
issuer namespace, store lineage, writer epoch, external target proof, and complete prior-ingress
fencing. Normal status and mutation use the immutable activation proof with offline verification;
retained candidate/completion reloads additionally resolve each signed historical store incarnation
against the local append-only registry, without a live provider or external registry call.

Every reservation attempt consumes a fresh committed pending-nonce observation. First use requires
equality with the qualified finalized floor. Later use allocates `local_high_water + 1` only when
the provider is equal or behind; provider-ahead or first-use mismatch returns typed divergence
without mutation. One incomplete stable intent owns the domain at a time. Candidate activation is
a contiguous bounded prefix, completion is canonical and permanent, and later runs can resume the
same intent independently of `run_id`.

Every active candidate carries the exact permanent operation key derived by `mfm-evm` from its
attested semantic reservation key and candidate ordinal. A parseable digest is not identity proof:
prefix validation, activation permits, capability ingress and settlement, broadcast ingress, and
PostgreSQL hostile-row qualification all rederive and compare that same key. An activation return
must equal the exact prepared attested candidate and operation key before it can advance state.

Every mutation consumes a fresh non-cloneable permit bound to the exact target, database session,
transaction, store lineage, and writer epoch. Deployment infrastructure owns target-held keys,
revocation, promotion, and sender-path fencing. Repository production assembly requires that
provider and has no fallback or self-attestation path.

The wallet authority has no non-rollback checkpoint either. Its append-only currentness rests on
immutable application-role history, schema qualification, and the provider's signed mutation
attestations, all of which live in the same database they describe. The limitation recorded under
*Append-only authority and the absent witness* below applies to the wallet domain identically.
An exact append retry whose batch is already durable may return `ExistingSame` after later
successors have advanced the stream: reconciliation must match the retried candidate to the
indexed current heads and must never rewind them to the retried predecessor. A configuration
append whose SQL commit acknowledgement is ambiguous retries only the identical canonical
revision, so the database resolves whether that exact append is already durable; divergence
remains an ambiguity or integrity failure.

### Append-only authority and the absent witness

PostgreSQL is currently the sole authority for append-only history. There is no witness process
outside the database that retains an independent record of a stream's head.

This is a deliberate, documented limitation rather than an oversight. A rolled-back database is a
valid earlier state of itself: heads chain, digests verify, and reduction is consistent at the
earlier revision. No predicate over the current state distinguishes "state at time T" from "state
at time T that was later restored", so detecting a rollback requires memory of a later head held
somewhere that did not roll back with it. Nothing in this repository holds that memory today.

The consequence is explicit: if a deployment restores the database from a backup, or otherwise
rewinds committed history, MFM will accept the rewound state and append onto it. The resulting
history verifies internally and cannot be distinguished from history that was never rolled back.
Detection of that class of failure is a deployment responsibility outside this repository.

Because that guarantee is absent, the invariant that must hold without exception is the one MFM
does enforce: no external effect may occur that was not durably journaled first. Every external
access is authorized by a committed record before invocation, and the runtime mints invocation
authority only after re-reading that exact committed authorization. Any change that lets an
effect run ahead of its journal record removes the last property this design still guarantees.

## Snapshot, export, and authorization boundaries

The indexed head is the first decision-bearing query in every PostgreSQL snapshot, so it
establishes the repeatable-read snapshot before any dependent query. The transaction then loads
the selected prefix, qualifies and reduces it, and requires the reduced head to equal the indexed head it read in
that same snapshot; it revalidates its exact target before commit. Configuration and run stream
identities are stable across successors; predecessor digests are compare-and-append preconditions
only.

Store ingress rejects any record that names an absent content-addressed object. Structural path
references and other semantic identities are not falsely treated as standalone objects; object
closure covers only values and history objects that the append actually retains. Both in-memory and
PostgreSQL backends invoke this same validator.

The portable encoder receives sealed canonical batch frames and purpose-specific fact routes. It
cannot enumerate raw `CommittedBatch` values through export evidence. Source prefixes are reduced
once at the maximum required producer head, with recursive graph, tenant, and route checks before
bytes are emitted. The encoder callback itself requires the workspace export-consumer seal, so a
caller cannot turn an opaque export fragment into a second purpose product.

Every qualified Read and Effect adapter receives the one-use committed authorization proof. It
must bind the access kind and retained physical certificate (and the exact state input for
stateful adapters) before entering its provider. An ordinary public Effect adapter call returns an
integrity fault. Wallet completion records retain a bounded canonical recovery closure so schema
validation remains within the generated frame budget while preserving rehashable public evidence.
Wallet activation and completion closures also retain the provider-issued mutation attestation
returned for the exact prepared mutation. The retained envelope carries the signed provider
challenge, target context, operation key, canonical payload digest, and signature; reload verifies
that envelope against the exact recovery preimage in a callback-free storage verifier, then performs
the separate historical-incarnation lookup against the append-only registry before accepting it.
The same persisted closure/proof boundary has a detached, callback-free audit witness that
reconstructs the completion mutation and public projection independently of that storage verifier;
it consumes an explicit provider trust snapshot and performs no PostgreSQL or live-provider IO.
Historical writer epochs are accepted only within the current store lineage, so promotion does not
invalidate already committed closures while same-lineage target, key, or attestation substitutions
fail closed.

## Application and transport surface

`mfm-app` owns qualified registry assembly, access policy, tenant isolation, configuration
resolution, admission, one-action drive, purpose-limited reads, and reviewed DTO rendering.
Each app-owned response has one private typed wire owner that validates its required version,
checked identities, exact variants, unknown-field exclusion, cross-field relations, and canonical
re-encoding before constructing the public byte wrapper; there is no generic response decoder or
cross-owner response type.
Normal app, CLI, and REST paths receive no raw database pool, registry administration, fence
issuer, signer secret, mutation permit, or generic invoker authority.

The public application operations remain `entry_points`, `check_ready`, `admit_run`, `drive_once`,
`read_public_run`, `replay_run`, `read_transition_trace`, `read_access_audit`, and `export_run`.
The CLI and REST crates are transport wrappers. Their standalone bootstraps fail closed because
this repository does not own a deployment fence provider; deployments embed the libraries and
inject a fully qualified application.

## Authority summary

| Surface | Authority |
| --- | --- |
| Authored or serialized program | Data only until exact certification. |
| Certified program document | Sole semantic program authority for admission and reduction verification. |
| Structured history writer | Non-cloneable run-mutation authority held only by Runtime. |
| Structured history reader | Cloneable purpose-read authority; each purpose returns only sealed purpose evidence. |
| Verified structured run | Qualified immutable history plus compact reducer state for one exact prefix; not a public purpose-read surface. |
| Purpose-sealed evidence | Public/trace/audit/replay/export newtypes that expose only that purpose's projection accessors. |
| Prepared/authorized/pending/committed access values | Private affine Runtime protocol stages. |
| Public physical certificate | Evidence only; cannot construct a live target session or mutation permit. |
| Qualified EVM routing catalog | Opaque deployment admission authority; serializable catalog data alone cannot construct it. |
| Wallet target session and mutation permit | Private deployment/provider authority, never serializable. |
| Content reference, DTO, cursor, or export | Identity or representation only; never bearer authority. |

## Dependency and deletion rule

There is one current structured implementation. The arbitrary execution graph, global ready-set
scheduler, generic executor, executor stores, compatibility readers, retired schemas, and their
workspace/task edges are absent. A diagnostic dependency graph may be derived from static
certified references, but it is not scheduling authority.
