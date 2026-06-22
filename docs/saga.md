# Certified Saga Contract

Status: authoritative companion to `docs/design.md` for saga remediation, scoped AC/DC language,
manual resolution, and cross-run resource claims.

Conceptual source: Michael Stonebraker, Xinjing Zhou, Peter Kraft, and Qian Li,
["Consistency and Correctness in Data-Oriented Workflow Systems"](https://www.vldb.org/cidrdb/papers/2026/p9-stonebraker.pdf),
CIDR 2026. That article motivates the AC/DC framing: durable workflow execution is necessary but
not enough; update-oriented workflows also need explicit error-handling, backout, compensation,
manual resolution, and concurrency-correctness semantics. MFM adopts that pressure as design
guidance but intentionally narrows the implemented claim to certified saga semantics unless a
resource is MFM-owned or replay-verifiable domain proof exists.

MFM implements certified saga semantics for typed external side effects. It does not claim full
AC/DC semantics for arbitrary external systems. Stronger AC/DC-style claims are honest only when
MFM owns the affected transactional resource or replay can verify a certified domain proof from
typed evidence. Core saga v1 records no AC/DC-equivalence proof; `Compensated` and
`ManuallyResolved` are saga outcomes, not independent external-world truth claims.

## Core Claim

A certified saga run is one append-only typed run whose certified spec, stream, artifacts, and
certificate prove one of these outcomes:

- forward execution completed with certified public-output evidence;
- confirmed forward side effects were remediated by linked remediation ledgers;
- an authorized manual decision resolved the run;
- the run failed without a compensation or AC/DC-equivalence claim under certified policy.

All saga decisions are derived from the certified spec plus append-only stream facts. Directive
selection, obligation state, run mode, and manual-block state are projections, not appendable
control events.

The kernel encodes saga authority in proof objects instead of repeated validators:

- `SagaAdmitToken` binds run-start and saga admission to the certified run id, spec hash, and
  policy.
- `CertifiedSideEffectContract` validates resource claims and side-effect contract evidence for
  live execution, resume, and replay.
- `SideEffectLedgerState` is the store-owned typestate view for legal side-effect ledger
  transitions.
- `ManualResolutionPrefixAuthority`, `ManualResolutionProofAuthority`, and
  `VerifiedManualResolutionForPrefix` bind manual authorization to the exact certified blocked
  prefix and retained proof artifacts.
- `SagaTerminalProof` is required to commit completed, compensated, manually resolved, and
  failed-without-claim terminal saga outcomes.

Manual-resolution terminalization rebuilds and re-verifies proof authority from the current
certified prefix plus retained evidence and authorization artifacts. Scheduler-local proof caches
must not authorize terminal saga commits.

## Certified Policy

`mfm-spec::v1::TypedExecutionSpec` carries hash-defining saga policy:

- `NoSideEffects`: lowering uses this when the forward graph has no side-effect nodes.
- `FailWithoutAcdcClaim`: after mutation, terminal failure may make no compensation claim.
- `ManualResolution`: after mutation, the run blocks for signed manual authorization.
- `CompensateCompleted`: confirmed forward side effects create remediation obligations; unresolved
  remediation follows the certified `on_remediation_unresolved` directive.

Remediation linkage is structural. Remediation nodes live outside the forward graph and are linked
to the forward side-effect node they compensate. Remediation reuses the side-effect protocol with
`SideEffectLedgerPurpose::Remediation { forward_ledger_key }`; forward ledgers use
`SideEffectLedgerPurpose::Forward`.

## Engagement And Quiescence

Saga handling engages at the first stream event that proves one of these facts:

- a non-retryable attempt or side-effect failure was recorded;
- a forward side-effect ledger became ambiguous.

After engagement, no new forward side-effect boundary crossings may be admitted. Recovery and
terminal evidence for already past-boundary forward ledgers may still arrive. Before classifying
obligations, runtime drives every past-boundary forward ledger to a quiescent phase such as
confirmation, not-submitted proof, failure, or ambiguity.
The store is the source of truth for this forward fence. Runtime can reject impossible scheduling
choices early, but every stream reader relies on store admission and projection rules.

Obligation classification is over the full current stream, not the engagement event prefix. With the
forward fence, all readers of the same stream derive the same obligation set and run mode.

## Run Modes

Public status reports semantic `RunMode`, not raw store phase:

- `forward`
- `remediating`
- `manual_blocked`
- `completed`
- `compensated`
- `manually_resolved`
- `failed_without_acdc_claim`

Attempt lifecycle is reported separately as attempt disposition: `started`, `completed`, `failed`,
or `interrupted`. `interrupted` terminalizes legal attempt bookkeeping for retry/recovery, but it is
not saga engagement, terminal run mode, compensation proof, or manual-resolution authority.

`Compensated` requires a non-empty owed set with every owed obligation closed by certified remedial
confirmation evidence. A clean failure with no past-boundary forward mutation resolves as
`FailedWithoutAcdcClaim`, never vacuous `Compensated`.

## Manual Resolution

Manual resolution is a signed authorization protocol, not a generic escape hatch. A
`ManualResolutionEvidenceSpec` cannot exist without an authorization policy:

- `evidence_schema`: schema for the operator evidence artifact;
- `authorization.verifier_id`: certified verifier identity;
- `authorization.signing_scheme`: digest-only manual-resolution signing scheme;
- `authorization.authority`: certified operator authority snapshot;
- `authorization.quorum`: required number of accepted operator signatures.

The old `operator_identity_ref_schema` manual spec shape and old operator-identity manual event
shape are invalid. There is no compatibility path for those formats.

Certification uses the live registry as authority for:

- schema role grants, including `manual_resolution_evidence` and
  `manual_resolution_authorization`;
- manual authorization verifier identities;
- operator authority snapshots and their digests;
- supported signing scheme and quorum shape.

The certified spec and certificate carry replay authority. Replay does not call a live signer,
verifier registry, certification registry, keystore, environment variable, or runtime signer source.

`ManualResolutionRecorded` references exactly two artifacts:

- `ManualResolutionEvidence`: the operator evidence artifact with certified schema and digest;
- `ManualResolutionAuthorization`: the canonical authorization proof artifact with certified schema
  and digest.

The proof artifact contains a canonical claim plus signatures. The claim binds:

- run id;
- certified spec hash;
- expected next stream sequence;
- stream prefix digest;
- manual block reason;
- unresolved obligations digest;
- selected manual outcome;
- evidence artifact schema, content hash, and artifact id.

`ManuallyResolved` means an authorized manual decision was recorded. It does not mean MFM
independently proved that the external domain state is correct.

## Resource Claims

Every side-effect contract declares a resource claim:

- `Exclusive`: pure preflight resolves a concrete exclusive key before invocation construction or
  live IO.
- `ExactTouchedSet`: adapter records exact touched-key evidence after execution.
- `ManualOnly`: MFM makes no framework-derived cross-run concurrency claim.

Only `Exclusive` takes a lane. `ExactTouchedSet` and `ManualOnly` do not acquire lane authority.
For `Exclusive`, runtime appends `StateAttemptStarted`, resolves all concrete lane keys through pure
capability preflight, and submits one `ResourceLaneClaimIntent` commit before invocation
construction. The store admits that intent only when every requested lane can be acquired together;
it materializes the committed `ResourceLaneClaimed` event, assigns the lane-local fencing token and
transition sequence, and records the matching `resource_lane_claim_events` and
`resource_lane_transitions` rows. A held lane blocks other ledgers for the same
`(namespace, key_schema_id, key, exclusive)` lane across runs.

Ordinary contention returns `ResourceLaneClaimBlocked`. That outcome parks the open attempt before
live IO; it is not a run event, not lane-transition authority, not a persisted read-model fact, and
never authorizes attempt, saga, or run terminal failure. Parked attempts retry with bounded backoff
and may wake on lane-release notifications, but notifications are only wakeups; the durable signal is
the append-only lane authority.

The no-deadlock invariant is structural: an attempt claims all required exclusive lanes in one
all-or-nothing claim commit and never holds one lane while issuing a second blocking claim. Lanes
release through committed `ResourceLaneReleaseIntent`/`ResourceLaneReleased` authority tied to the
active claim and fencing token. Remediation ledgers acquire and release lanes under the same rules as
forward ledgers.

Exact touched-set evidence is schema-checked at admission and replay, but the kernel does not infer
domain isolation from it without a future domain verifier.

Forward side-effect ambiguity is itself saga-engaging evidence. The store admits it only when the
same commit also records the non-retryable attempt failure that explains why the run is entering
saga handling; standalone ambiguity evidence is rejected.

## Layer Authority

`mfm-store` remains append-only and structural. It owns envelopes, sequence numbers, ordinals,
logical keys, artifact evidence admission, stream-derived preconditions, and projections. It does
not certify schemas, verify signatures, resolve operators, or decide domain truth.

`mfm-runtime` owns spec-aware saga advancement. It rebuilds verified history from the run stream,
derives engagement, quiescence, obligations, remediation order, and manual-block state, then
constructs guarded commits from certified runtime authority.

`mfm-replay` verifies from the certified spec, certificate, stream, retained artifact evidence, and
retained proof bytes only. A historical manual resolution is rejected unless the prefix derives
`ManualBlocked`, artifacts match the event and certified roles, the proof claim exactly matches the
event and prefix, signatures verify, the signers are in the certified authority snapshot, and quorum
is satisfied.

`mfm-app`, CLI, and REST expose status and assembly surfaces only. Public status may show required
manual evidence schema, signing scheme, authority id, allowed operator public identities or a safe
summary, and quorum. It must never expose signer runtime sources, keystore paths, password paths,
passwords, mnemonics, private keys, RPC authorization material, or secret-bearing environment
variables.

## Deferred

The following remain outside the current certified saga contract:

- full AC/DC-equivalence claims;
- domain verifier APIs for commutativity, escrow, predicate snapshots, finality, or isolation;
- per-obligation manual targeting;
- generic retry/replan/continuation policy;
- cancellation semantics against remediation;
- lane fairness, queueing, deadlock detection, or global scheduling policy.

Lane fairness and queueing remain outside the current certified saga contract; callers must not
assume starvation freedom under sustained single-lane contention.
