# MFM Design Contract

Status: authoritative structured-runtime design contract

This document defines the one current runtime, history, storage, replay, and application design.
The exact recoverability encodings live in `contracts/recoverability/v1/annex.json` and
`contracts/recoverability/v1/corpus.json`; those generated contracts are authoritative for bytes.

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
- The store owns the sole callback-free fold and is authoritative for hostile persisted input.
  No mutable cursor row and no second reducer exist.
- `RunClosed` shares the append that first makes the root outcome derivable, including a
  zero-state `RunAdmitted + RunClosed` append. No record is legal after closure.
- Runtime owns the only production path that can request semantic run-history mutation through
  `RuntimeHistoryPort`. The store's private adapter owns the sole fold and append authority.
  Runtime performs at most one semantic transition or one audited external-access operation per
  `drive_once` call. Callers never supply a trusted run digest for admission; the store derives
  `RunId` from the annex `mfm.run-id-preimage.v1`.
- Authority-bearing integration traits (`RuntimeHistoryPort`, physical-binding verification,
  wallet authority, checkpoint authority, and deployment credential issuance) require a
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
- Replay and every read projection use the store fold and perform no callback, provider, signer,
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

The generated recoverability annex allows canonical JSON values and retained history frames up to
32 MiB. These ceilings remain separate from the smaller string, portable-export, and
provider-proof budgets: they admit the largest qualified structured-program value while keeping
every transport and retained payload bound explicit.
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
A waiting Read may expose a later lane; an unresolved barrier stops further scanning. Joins retain
declaration order independent of physical completion order.

## Run history and fold

`mfm-journal` owns strict data and identities. `mfm-store` owns legality and continuation. The fold
validates canonical objects and hashes, certified-program closure, exact predecessor and writer
lineage, record logical-key uniqueness, cursor legality, lexical provenance, access linkage,
attempt ordinals, physical-binding evidence, fact closure, terminal closure, and the exact object
set introduced by each append.

The fold derives:

- initial and current lexical bindings;
- declaration-ordered actionable occurrences and active fan-out lanes;
- outstanding Read and Effect access;
- semantic and physical journal heads;
- waiting, possible-entry, integrity-blocked, and closed status;
- the exact terminal root outcome; and
- complete records and append heads in physical chronology.

Public-read and recorded-replay purpose projections retain only the
fold-derived `RunEvidenceStatus`; the actionable frontier and its capability,
input, and state references remain internal to the fold and Runtime adapter.

The explicit offline replay entry point consumes raw history plus concrete trust
and returns an opaque `OfflineVerifiedRun` containing only recorded status and
bounded export metadata. The complete verified cursor, objects, and bindings
never cross the store boundary.

Incremental mutation returns a successor produced by the same fold state. Refolding every complete
prefix from raw persisted batches must produce an equivalent verified run.
The private production adapter may retain at most one verified successor, including a non-mutating
frontier result, for the next Runtime drive. Before reuse it compares that cached journal head with
the backend's indexed current-head projection; a mismatch or a cache miss performs the authoritative
full fold. The exact-head projection is the snapshot point for a non-mutating load; any later append
is handled by the append path's exact-head compare-and-append. The one-entry cache is therefore only
a bounded replay-cost optimization and never a source of store authority.

Production physical qualification binds an exact purpose tuple: access kind, semantic capability,
semantic adapter, qualified adapter implementation, stable resource lineage when applicable, and
an ordered immutable physical-target release history. Each release retains its full public
certificate, admitted routing policy, exact target identity, predecessor certificate, and, for an
Effect successor, the public lineage head that activated it. Callback-free verification accepts
any retained release while refolding recorded history, but newly proposed access must select the
current release. A refreshed Effect authorization must be a strict descendant of its exact prior
certificate at the folded non-rollback head; supersession evidence proves that same old-to-new
edge. Reconstructing a process with a longer retained history therefore preserves old runs while
preventing new authorization against superseded releases.

Memory and PostgreSQL implement the same store contract. PostgreSQL writes real SQL transactions,
uses exact-head compare-and-append, and opens only after a deployment-supplied authoritative writer
fence qualifies the store scope and epoch. An unavailable or unqualified database fails closed;
there is no memory fallback.

## Runtime access choke point

Runtime interprets the minimum declaration-ordered actionable path from the verified cursor. Its
private access bracket is:

```text
Prepared<K>
  -> committed ExternalAccessAuthorized
  -> Authorized<K>
  -> exactly one registered invoker entry
  -> PendingObservation<K>
  -> committed ExternalAccessObserved
  -> CommittedObservation<K>
  -> exact registered state settlement
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
`EntryUnknown` parks possible entry. Committed `IntegrityFault` blocks. An unmatched Read waits.
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
`IntegrityFault` can fold to durable `BlockedIntegrity`. Public projections may expose the phase,
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

The registry builder privately installs that exact capability/adapter pair as the sole kernel
process baseline. It is retained for Runtime dispatch but excluded from entry support closures that
do not use fact selection. Application assembly cannot register or shadow it, and the baseline does
not relax rejection of any other unused process binding.

Authorization atomically records the current tenant fact frontier without advancing it. Only a
directly acknowledged new authorization mints the store's non-cloneable one-use scan permit;
idempotent content and acknowledgement recovery do not. The permit exposes only a complete bounded
scan through the captured frontier. The scanner follows every dense route, verifies each producer
with the callback-free fold, applies the admitted source manifest, excludes the consumer run, and
uses the fixed deterministic top-k evaluator. It cannot append history or obtain generic store
query authority.

A positive response retains the request digest, exact source references, canonical selected
subject/response/claim bytes, deterministic identities, and an authorization/frontier completeness
attestation.
Exhausted bounds and backend unavailability are typed safe failures; missing, malformed, or
substituted provenance is an integrity fault. Public verified loads recompute every retained
positive response at its recorded frontier and reject any mismatch, so replay and projections never
treat a merely well-shaped completeness claim as authoritative.

## Replay and public projections

`mfm-replay` owns the portable export format and offline verifier, and projects the reviewed public
run view, transition trace, access audit, and replay summary from the sole store fold. Pages are
fixed to one journal head. Portable export first authorizes every recursively referenced prior-run
source under the sealed export purpose and only then serializes exact committed-batch envelopes,
source relationships, and semantic/physical fixation with the exact target key, database identity,
fence generation, release epoch, and current-incarnation reference with a closure reference; denied or incomplete
source closures emit zero bytes. Offline verification uses only bundle bytes and an explicit trust
snapshot against the store's read-only fold entry. The program, retained-release, and external
checkpoint verifiers used by that snapshot are workspace-sealed deployment authorities; an
ordinary consumer cannot substitute callbacks that accept forged fixations. Current portable exports use a bounded
newline-delimited frame stream with media type
`application/vnd.mfm.structured-run-export-stream.v2`; each canonical frame carries an ordinal,
kind, payload, and predecessor digest, and a terminal seal binds the exact closure, fixation,
per-run authenticated principal, fixed `export` grant, content-addressed policy-decision
references, counts, bytes, and chain. Legacy monolithic JSON objects and retired bytes are
rejected.

Authorization discovery uses the requested kind's exact root cutoff and recursively advances each
producer only to the maximum transition head required by selected fact routes. A later audit-only
suffix therefore cannot add a semantic-export dependency. Decision references in the terminal seal
are opaque content-addressed policy evidence; offline consumers do not resolve them live, and the
explicit trust snapshot must bind the exact closure digest before folding.

Recorded replay is verification-only: it folds the committed prefix and returns its bounded
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

Every mutation consumes a fresh non-cloneable permit bound to the exact target, database session,
transaction, store lineage, and writer epoch. Deployment infrastructure owns target-held keys,
revocation, promotion, and sender-path fencing. Repository production assembly requires that
provider and has no fallback or self-attestation path.

The provider child never owns the non-rollback checkpoint. A distinct external checkpoint
authority retains the exact physical target, fence lineage, epoch, acknowledged database-prefix
digest, and at most one prepared successor across child crashes. `Prepared` records the exact
predecessor, successor, operation, and optional successor target before SQL mutation;
`Acknowledged` advances only after the database exposes that exact successor prefix. Startup at a
prepared predecessor retains the preparation and permits only its byte-identical retry; startup at
the prepared successor acknowledges it. Rollback, database-ahead state, a competing successor,
or any target/incarnation/public-head mismatch rejects readiness without repair or fallback.
An exact append retry whose batch is already durable may return `ExistingSame` after later
successors have advanced the stream: reconciliation must match the external run and fact heads to
the indexed current heads and must never rewind them to the retried predecessor.
Because SQL commit precedes external acknowledgement, a concurrent read may briefly observe a
valid indexed prefix one checkpoint step ahead; PostgreSQL run and configuration read paths retry
that exact transient `InvalidHistory` observation a fixed number of times and still fail closed
with the final error. A configuration append whose SQL commit or checkpoint acknowledgement is
ambiguous reconnects through the append authority and retries only the identical canonical
revision; a durable prepared successor is acknowledged, while divergence remains an ambiguity or
integrity failure.

## Snapshot, export, and authorization boundaries

The indexed checkpoint head is the first decision-bearing query in every PostgreSQL snapshot. A
repeatable-read transaction then loads the selected prefix and validates its exact external target
and checkpoint successor before commit. A bounded retry covers only the commit-before-acknowledgement
window; persistent mismatch remains an integrity failure. Configuration and run stream identities
are stable across successors; predecessor digests are compare-and-append preconditions only.
Configuration resolution uses the same bounded load classification as append preparation, so the
application admission path cannot turn a transient checkpoint race into a false missing or
malformed configuration result.

Store ingress rejects any record that names an absent content-addressed object. Structural path
references and other semantic identities are not falsely treated as standalone objects; object
closure covers only values and history objects that the append actually retains. Both in-memory and
PostgreSQL backends invoke this same validator.

The portable encoder receives sealed canonical batch frames and purpose-specific fact routes. It
cannot enumerate raw `CommittedBatch` values through export evidence. Source prefixes are folded
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
that envelope against the exact recovery preimage and resolves its full signed physical incarnation
against the append-only registry before accepting it. Historical writer epochs are accepted only
within the current store lineage, so promotion does not invalidate already committed closures while
same-lineage target, key, or attestation substitutions fail closed.

## Application and transport surface

`mfm-app` owns qualified registry assembly, access policy, tenant isolation, configuration
resolution, admission, one-action drive, purpose-limited reads, and reviewed DTO rendering.
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
| Certified program document | Sole semantic program authority for admission and fold verification. |
| Structured history writer | Non-cloneable run-mutation authority held only by Runtime. |
| Structured history reader | Cloneable callback-free purpose-read authority; each purpose returns only sealed purpose evidence. |
| Verified structured run | Fold-derived internal semantic and chronology authority for one exact prefix; not a public purpose-read surface. |
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
