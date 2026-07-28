# mfm-runtime

Typed kernel crate for the event-sourced typed state-machine workflow runtime.

`docs/design.md` is the normative typed-core authority contract. This crate executes only certified
typed execution specs.

`CertifiedRuntimeSpec::new` accepts only `mfm_certify::CertifiedTypedSpec`. Runtime callers cannot
construct runtime authority from a parsed `TypedExecutionSpec`, a hash-only
`mfm_spec::v1::HashedSpecEnvelope`, or parsed persisted spec/certificate data. Persisted
spec/certificate bytes must pass through the certifier verifier before they can reach this crate.

Before admission, `CertifiedRuntimeSpec` owns the one `CertifiedTypedSpec` plus disposable
position-and-ID indexes. It is non-cloneable. Once the admission journal is loaded,
`verify_current_run` consumes that owner: the certified authority moves into the store
`VerifiedRunView`, while `VerifiedCurrentRun` retains only the non-authoritative indexes beside the
view. Post-admission code obtains a borrowed `CurrentRuntimeSpecRef` from that pair; it never keeps a
second envelope, certificate, graph, node, cell, or descriptor authority.

External reads use one generic runner contract. Before admission, runtime awaits the adapter's
asynchronous process-local ingress validation; adapters must move blocking resource discovery off
the async worker. Runtime then loads the typed config, arbitrary certified input tree, and certified
context; asks the state for its immutable plan; lets the adapter execute only that plan; invokes the
state reducer; and stages exactly one typed primary read-evidence artifact plus any kernel-owned
fact-query evidence and the canonical output. Replay performs the same materialization and
reduction from retained evidence without constructing live capabilities.

Ordinary pure states likewise use one private generic runner registered under the exact `pure`
factory. The caller supplies retained-artifact authority, the registry-minted factory binding, and
an optional typed context-output extractor. Runtime loads certified config, the complete input
tree, and certified context, invokes `PureState::run`, and stages the canonical output; domain
adapters do not implement a second pure runner.

`ErasedRunnerRegistry` is created with one explicit `ExecutableIdentityTemplate`. It mints and
validates every runner, framework, and adapter binding from the template: factory ids differ, but
all bindings preserve the same caller-attested executable-byte digest. Runtime has no default or
label-derived executable identity.

The visible runtime model is:

```text
certified spec + store-owned VerifiedRunView
  -> bound runtime context
  -> deterministic frontier scheduler decision
  -> attempt lifecycle
  -> sealed runner invocation
  -> guarded commit
```

The certified spec is the static certified transition graph. The store loads one committed journal
with its exact retained objects and validates its physical/current-format structure. That physical
load does not verify historical manual-resolution signatures. Consuming the journal with the exact
`CertifiedTypedSpec` runs the sole store-owned semantic certified-history fold, including
deterministic `mfm-manual-auth` verification against certified replay authority, and mints a fully
authorization-verified `VerifiedRunView`. That semantic pass performs no live operator, signer, or
keystore lookup and does not decide external truth.

Runtime borrows the non-cloneable view and trusts its derived fold. It does not reconstruct or
reverify historical manual proofs and does not own a copied stream, projection snapshot,
retained-object map, duplicate historical validator, or second history fold. After an append,
successor verification consumes the old view and the newly loaded journal, checks exact prefix
extension, semantically verifies the suffix, and moves the same certified authority forward. The
bound runtime context proves runner binding availability and executable identity for every
certified executable node before run admission or resume dispatch, and rejects context-bound output
nodes whose runners do not expose a context-output extractor. Runner invocation is sealed by
runtime-owned input and certified-context materialization, runner identity checks, and capability
scoping. The commit planner verifies typed payloads, side-effect protocol rules, staged artifacts,
context-bound output evidence, references, retention bindings, and commit preconditions before
building purpose-specific `PreparedCommit<Purpose>` values and submitting them through
`PreparedCommitPlan`.

Current lifecycle algorithms temporarily consume purpose-specific borrowed readers from
`mfm_store::v1::current_lifecycle`. Those readers do not expose raw journal authority and must not be
stored, cloned, serialized, or promoted into a runtime projection. The complete audited lifecycle
cutover deletes this temporary reader module.

When runtime refreshes history, it consumes the old view with a newly loaded committed journal.
The store carries certified authority forward only after proving a strict extension with an exact
unchanged prefix and unchanged old objects and fully validating the semantic suffix; runtime does
not recertify, reverify historical authorization, or merge two histories.

Launch is a pre-FSM admission lifecycle, not a scheduler-dispatched state attempt.
`RunAdmissionLifecycle` verifies the certified spec, launch artifacts, seeds, configs, executable
availability, capability availability, and binding digest before minting the single-payload
`RunAdmitted` commit. No state attempt, cell production, artifact-reference payload, or retention-ref
payload is appended during admission. Ordinary state attempts, public-output rendering, retention
projection, and terminal framework work then use the shared started-before-run authority path.
`RunCompleted` is derived from the sealed `CompleteRun`
framework state for successful public-output completion, or from `ResolveSagaTerminal` when the
saga terminal path resolves compensation, manual resolution, or failure without an AC/DC claim.

Module roles:

- `spec_authority`: pre-admission certified owner and disposable positional runtime indexes
- `binding`: bound runner identity and executable availability for a certified runtime spec
- `admission`: pre-FSM run-start admission lifecycle
- `history`: borrowed algorithms over `VerifiedRunView` and context-aware input materialization
- `frontier`: pure scheduler decision and attempt planning
- `transition`: closed transition decisions over certified spec and verified history
- `attempt`: ordinary attempt lifecycle from selection through terminal commit
- `framework_lifecycle`: started-before-run framework attempt lifecycle for public-output rendering,
  retention, and terminal states
- `recovery`: open-attempt recovery frontier validation
- `side_effect_lifecycle`: side-effect attempt uncertainty and recovery evidence guards
- `scheduler`: serial orchestration over admission, transition, lifecycle dispatch, and store
- `invocation`: sealed runner context builder, certified context handles, and materialized inputs
- `commit`: launch and runner-output commit planning
- `side_effects`: runtime protocol guards for durable side-effect ledgers
- `framework`: certified framework lifecycle states
- `artifacts`, `runners`, and `error`: artifact evidence, runner registry/types, and runtime errors
