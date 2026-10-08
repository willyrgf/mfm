# Experiment: validate the refactor simplification architecture

**Status:** execution started; baseline and predictions recorded before specimen implementation.
**Architecture baseline:** [RFC_REFACTOR_SIMPLIFICATION.md](RFC_REFACTOR_SIMPLIFICATION.md),
including its projection correction at `d72b2e08`.
**Available evidence:** the source-review finding F1 in section 6 below.
**Purpose:** establish whether the disputed generic/concrete boundaries justify proceeding into
detailed design, and identify precisely which claims remain provisional.

This document owns the experiment procedure and its evidence record. The RFC owns the proposed
architecture; [docs/design.md](docs/design.md) and [docs/architecture.md](docs/architecture.md)
remain authoritative for the implemented system. The experiment does not certify the whole RFC,
freeze public API names, or implement its production cutovers.

## 1. Questions and falsifiable hypotheses

Write the expected observations before implementing the specimen. Use these four hypotheses to
select work; do not expand the experiment merely to complete the RFC's acceptance matrix.

| ID | Architectural hypothesis | Independent evidence | Counterexample requiring a design decision |
| --- | --- | --- | --- |
| H1 | Direct typed construction and private factory erasure can associate real State, adapter and resource owners without structural source lowering. | A changing-type consumer compiles and runs; incompatible adjacency/Observation pairing fails compilation; a conflicting owner rejects before attachment or IO. | Required behavior needs source expansion, an erased public builder, serialized Observation or ambiguous owner substitution. |
| H2 | A genuinely different native owner can implement a shared business contract locally; compatible instance changes remain data. | An independent author adds an owner with different Binding/Receipt/Fault/resource types; the shared State and generic execution/persistence code acquire no protocol branch. Another binding/policy reuses its installed factory. | Native meaning is coerced, or adding the owner requires central protocol dispatch, a universal envelope or a copied support matrix. |
| H3 | Canonical originals and factory-selected code provenance suffice for interpretation without another stored projection. | Literal expected canonical bytes/values match provider arguments and complete consuming output; original encoding occurs once; a non-Serde Observation works; original and decoded Receipt correspond. | Hot execution uses pre-encoding terms, output reconstructs an original, selected code identity is fabricated, or observation-only specializations become indistinguishable. |
| H4 | One semantic Effect can own progression while native custody retains exact authority privately. | Command acknowledgement precedes native IO, settlement acknowledgement precedes interpretation, and interrupted recovery keeps the same command/ref/EffectId and retained winner. | Safety requires public preparation/reservation States, another independently prepared winner, refreshed terms, premature IO or reconciliation after acknowledged settlement. |

One failure does not imply abandoning the entire architecture. If two protocols have different
business semantics, H2's sharing proposal fails and they need distinct contracts. If a normal
prototype bug causes a failure, correct that bug and rerun its affected case. Revise the boundary
when the required behavior cannot fit without restoring the complexity the RFC intends to delete.

## 2. Scope, controls and integration seam

Use one disposable specimen with a construction/Read path and a separate Effect fragment. Add
only the provisional types and wiring needed to challenge H1-H4. Reuse actual canonical values,
Runtime, Journal and Store; do not build another engine or product.

| Existing control | Use in the experiment | Limit of existing evidence |
| --- | --- | --- |
| [Lifecycle consumer](crates/domains/evm/tests/lifecycle_runtime.rs) | Observe -> Validate -> Report with real changing endpoint types and complete lifecycle output. | Current authoring/native interfaces do not prove the proposed replacements. Scripted dataflow does not prove chain or signed-wire IO. |
| [Portfolio native boundary fixture](crates/domains/portfolio/tests/native_boundary.rs) | Source of structurally different native query, receipt, binding and fault semantics for an extension challenge. | Reuse relevant facts, not its Prepare/Confirm injection scaffolding. It is a test owner, not supported production chain code. |
| [Fresh ownership controls](crates/kernel/program/tests/claim_conflicts.rs) | Starting point for one deliberately conflicting owner and its observable rejection. | Current checks do not certify the proposed Catalog or target descriptor. |
| [EVM semantic recipes](crates/domains/evm/src/transaction/recipes.rs) | Actual DeploymentRequest and DeployedContract derivation of nonce-free native commands. | Two semantic commands must share private custody without a new translator registry. |
| [Native transaction controls](crates/live/evm/src/transaction_tests.rs) | Existing authority/winning-wire/cancellation premises to inspect and reuse where unchanged. | Current stage decomposition cannot prove one-Effect orchestration. |
| [Managed Effect fixture](crates/live/evm/tests/evm_contract_effect_e2e.rs) | Focused physical evidence when a changed authority assumption requires it. | Reconstruction retains the keystore owner; it is not process-restart signing support. |

Before coding, record how candidate callbacks will enter the existing Program/Runtime boundary:

1. A small direct target association path may provide evidence for the exact mechanism exercised.
2. A narrowly fenced bridge to the current Program representation may provide typed-port and
   scheduling evidence, but target wire identity and complete cold restoration remain unvalidated.

A builder that lowers through AuthoringSource/Inject*/NativeAbi does not demonstrate their removal.
Existing wire descriptors may hide assumptions that the proposed descriptor removes. Label every
result obtained through that bridge accordingly; do not infer target ABI correctness or deletion
from successful old-wire execution.

The specimen is exploratory, with no supported production API or compatibility reader. If reaching
the real boundary requires another Runtime, widespread consumer cutover or substantial duplicate
machinery, stop and record that obstruction. Investigate the unresolved design separately rather
than expanding this experiment into the implementation.

The ZEC -> Ethereum WBTC -> Aave example remains a paper exercise. Live route discovery, wallet
custody, Aave deployment, cross-run ticket exclusivity and production finality are outside scope.
Full target-wire restoration, all checkpoint/shadow-owner cases, retained-selection races, every
resource-free RunView state and backend acceptance remain
[RFC section 16.2](RFC_REFACTOR_SIMPLIFICATION.md#162-implementation-acceptance), unless a specific
counterexample makes one isolated mechanism necessary here.

## 3. Ordered procedure

### 3.1 Trace actual consumer requirements

Inspect Portfolio snapshot/enrichment and the lifecycle before selecting the shared native port.
Record a short trace for each relevant Read and Effect:

```text
Read success/domain outcome:
  prepare/check request -> native IO -> admit/decode original
  -> fused projection/interpretation -> acknowledge original and outcome

Effect settlement:
  prepare/check command -> acknowledge command -> native IO
  -> admit/decode/qualify original -> acknowledge settlement
  -> pure reprojection/interpretation -> acknowledge outcome

Operational fault:
  admit original fault -> acknowledge original -> classification/recovery
```

Check the trace against [the Runtime engine](crates/kernel/runtime/src/engine.rs) and the proposed
ports. Read completion has no intervening settlement acknowledgement; Effect interpretation does.
For every edge, name the owner, available value/Object/ref, required causal facts and permissible
repetition. This remains a requirements/ownership review, not an alternative scheduler.

Resolve these consumer questions explicitly:

- If a collection stops halfway through, may its unfinished sources repeat, or does the product
  require individually acknowledged source results? Existing RPC stages alone do not answer it.
- After uncertain transaction broadcast, which authority facts prevent another independently
  prepared transaction? Identify their custody and their acknowledgement meaning.
- Which native originals and selected code identities must the lifecycle output retain? Preserve
  those actual requirements when choosing projection inputs.
- What are the shared units, identity, observation-point, trust and rejection semantics? A second
  receipt format is not evidence that the business meanings match.

Expected results must come from those requirements and literal fixture premises. Existing product
mislabels or helper-derived expected values do not establish correct units or outcomes. Record
unresolved requirements as unvalidated; do not invent a consumer requirement to make the port fit.

**Exit:** H1-H4 have explicit expected facts and failure consequences, and the chosen consumer has
no unresolved ownership contradiction hidden by the specimen. Paper evidence does not establish
Rust expressibility, native truth or physical durability.

### 3.2 Compile and execute the construction/Read specimen

Use the actual ConfiguredContract -> ObservedConfiguration -> ValidatedConfiguration ->
ContractDeploymentReport path. Keep the complete output and its native-original/code provenance.
Implement only enough provisional consuming builder, typed factory association and private
callback erasure to exercise that path.

| Probe | Required observation | Evidence to retain |
| --- | --- | --- |
| Changing Current | Each append consumes the prefix and accepts only the declared input type; caller retains borrowed initial input. | Consuming source and focused compiler output; changed input rejects independently at Runtime. |
| Incompatible ports | Wrong adjacency and State/adapter Observation pairing fail compilation. | A small consuming compile-fail case with the intended diagnostic; do not assert incidental compiler wording. |
| Plain Observation | Observation has no Serde/MfmValue requirement and includes a non-Sync field consumed inside the pure callback. | Compiling factory/callback path and output; do not add unused bounds just to satisfy old machinery. |
| Conflicting owner | A different concrete owner claiming the selected contract rejects before attachment/provider invocation. | Known conflicting claims, rejection cause and independent attachment/IO instrumentation. |
| Native versus local failure | A native fault retains its cause and operational meaning; local binding disagreement returns invocation failure with zero provider calls and no operational append. | Predetermined fault/cause facts, provider events and independently inspected Store state. |
| Canonical authority | A normalizing serializer makes the decoded admitted value differ from the pre-encoding Rust field. Checks/projection use the admitted value. | Literal expected JSON/value defined before coding, original serializer count, arguments and retained bytes/ref. |
| Complete original retention | Actual consuming output retains the supplied admitted Object and selected leaf ref. | Exact output originals compared with independently observed admitted originals; no Receipt serializer retry. |

For the normalization probe, use a controlled fixture whose Rust field is 9 and canonical field
is 7. Define the complete fixture JSON and expected output before implementation. This is a codec
boundary probe, not an alteration to production native evidence. Merely comparing two projector
outputs is insufficient; both could be using the wrong representation.

If the seam permits reconstruction, recreate callbacks and interpret retained facts without native
IO. Report whether reconstruction used the current wire or the target association mechanism.
Passing current-wire hot/cold equality does not validate target-wire specialization identity.

**Exit:** H1 and the exercised parts of H3 have independent evidence or a precise obstruction.
API names remain provisional; do not add convenience layers after the discriminating cases pass.

### 3.3 Independently add a native owner

After the first owner works, give a different author the documented semantic and extension ports.
Agree on the common observation meaning before implementation. Prefer the existing Portfolio
native fixture's distinct facts where they fit a genuine holdings contract; do not force that
fixture into the lifecycle scalar contract merely to avoid another consuming Read.

The second implementation must differ in native binding, receipt, fault and resource ownership,
and exercise those differences. Renaming an EVM wrapper does not qualify. Keep it within the same
specimen and generic spine; add only the shared semantic Read necessary for this challenge.

Record the exact files/imports changed for:

- A second compatible binding/policy instance of the first adapter.
- A structurally different owner of equivalent business semantics.
- A semantic mismatch that is rejected or deliberately uses a distinct State/contract.

**Pass:** compatible instances are declaration data; the new owner's changes stay in its native
implementation/contribution and composition; the genuinely shared State and generic association,
Runtime, Journal and Store acquire no protocol dispatch. Native facts and causes remain distinct.

Also challenge the identity assumption removed with Observation schemas: use equal remaining
stored contracts and equal family IDs with behaviorally different Observation specializations.
Expose the collision, then exercise distinct committed semantic identities or checked instance
facts. Simply giving unrelated fixture types different IDs and comparing hashes proves little.
Record whether the target descriptor/association was actually exercised. Revision discipline and
executable-byte authenticity are not established by this fixture.

**Exit:** H2 has independent extension/change evidence. H3's specialization claim is supported only
to the extent the target identity mechanism was exercised; otherwise retain it as unvalidated.

### 3.4 Challenge one semantic Effect and partial outcomes

Use actual DeploymentRequest and DeployedContract commands through the same private native
custody routine. They may be separate small fragments in this specimen; do not rebuild the whole
lifecycle just to connect them. Before execution, compare the current and proposed lineage:

| Identity/fact | Record before relying on existing controls |
| --- | --- |
| Logical authority | Current reservation/preparation/transaction EffectIds versus the proposed semantic EffectId. |
| Command correspondence | Acknowledged semantic command ref, derived native-command ref and the exact terms each commits. |
| Physical authority | Reservation keys, nonce domain, epoch, first-winner rule and ambiguous-load meaning. |
| Winning wire | How retained bytes are qualified, acknowledged and selected before broadcast. |

Stage removal changes the origin of orchestration identity even if the SQL functions are unchanged.
Exercise native helper correspondence; do not assume that moving calls preserves it.

Use this main scripted schedule:

1. Commit the command but withhold the append response. Observe the underlying Store independently.
   Adapter, authority, signer and provider IO remain at zero until acknowledgement reaches Runtime.
2. Release command acknowledgement and produce a qualified native settlement using retained
   command/wire authority. Record the exact command, native ref, EffectId and winning bytes.
3. Commit settlement but withhold its append response. Cancel the caller, then reconstruct and
   resume through Runtime's retained-fact qualification.
4. Interpret the retained acknowledged settlement. Its original/code provenance remains exact;
   reconciliation, signing and submission are not invoked during acknowledged interpretation.

The observer must bypass the scripted response wrapper. Inspect exact underlying Store bytes/head
and the independently instrumented native sink, not two views derived by the same projector.
Withheld responses establish caller behavior under that schedule; they do not establish a real
database/network failure or recording of an interrupted physical attempt.

Run one actual readiness/partial-outcome case: after acknowledged deployment, make checked addition
overflow. Retain the domain refusal and successful deployment; configuration preparation remains
uninvoked. A legitimate refusal must not become an Internal preparation failure.

**Fail:** premature IO, interpretation before settlement acknowledgement, changed command/winner,
refreshed terms, discarded prior exposure, or native reconciliation during acknowledged
interpretation. Count logical commands, reservations and retained transaction identities;
repeated queries or retransmission of identical retained bytes are permitted.

If authority keys/domain/epoch, identity mapping, first-winner transactions, ambiguity, signer
lifetime or retain-before-broadcast assumptions change, select one focused managed physical
experiment for that changed guarantee. A scripted Store cannot prove PostgreSQL atomicity,
synchronous durability or real broadcast behavior. Existing physical results are controls only
where the actual contract remains unchanged. Retaining one keystore owner during reconstruction
does not certify signer process-restart support.

**Exit:** H4 has bounded scheduling/lineage evidence or a counterexample. Physical claims without
the necessary evidence remain unvalidated; full backend/custody acceptance belongs to the cutover.

## 4. Evidence, commands and reproducibility

For each execution record the exact candidate/control commit, any uncommitted diff, integration
seam, selected native identities, complete fixture expectations, command/exit status and evidence
location. Conclusions about an older control do not transfer to a changed candidate automatically.

[docs/build-and-verification.md](docs/build-and-verification.md) owns command selection;
[nixfied.nix](nixfied.nix) owns managed fixtures. All direct Rust tools use the default Nix shell.
Once real probe targets exist, choose the narrowest commands exercising them, for example:

```sh
nix develop -c cargo check -p <affected-package> --all-targets
nix develop -c cargo test -p <affected-package> <probe-filter> -- --nocapture
```

These are templates, not executable experiment IDs or recorded passes. Select existing managed
orchestration if a physical probe is required. Broader gates and final CI follow the actual change's
verification scope; do not require the entire implementation matrix to decide an isolated premise.
Follow existing diagnostic/secret-custody rules when recording evidence; do not capture keys,
credentials or full private requests as trace context.

Review independent oracles before interpreting results. Literal fixture quantities/bytes, actual
provider arguments, underlying Store records and authority facts must discriminate the proposed
behavior. Test counts, code coverage, matching hot/cold projections and prototype LOC alone do not.

## 5. Decision and completion criteria

Use three evidence conclusions for each claim:

| Conclusion | Meaning | Next action |
| --- | --- | --- |
| Supported | The named hypothesis survived the specified discriminating experiment and its independent oracle. | Proceed with that design decision, retaining its implementation acceptance obligations. |
| Counterexample | Required behavior contradicts the proposed boundary or representation. | Revise the affected RFC contract and the experiment before broader design. |
| Unvalidated | The experiment did not run, the oracle/seam was inadequate, or the claim lies outside the exercised mechanism. | Keep that decision provisional and name the missing evidence. |

Separate evidence about typed ports, original custody, scheduling, target association and physical
durability. A successful ports experiment cannot approve an untested target wire or native custody.

The independent reviewer checks changed modules/imports, public concepts, factory counts,
duplicated facts, configuration points and future change sites. Report specimen LOC separately
from an eventual production added/deleted/net delta. No production reduction is demonstrated until
the coherent cutover deletes superseded code. Extension locality does not prove throughput;
workload/concurrency/run-size requirements are needed for that separate conclusion.

Stop after H1-H4 have discriminating evidence or precise obstructions and each conclusion names its
limits. Extract the result record, then discard the disposable specimen and integration harness.
Production regression tests can be retained during an authorized complete cutover; no experimental
construction path becomes a second supported API. Preserve current causal, ambiguity and custody
contracts throughout the experiment, following [AGENTS.md](AGENTS.md).

## 6. Findings and current results

### F1: projection must supply existing original and selected code provenance

**Method:** source/contract inspection; no executable prototype.
**Evidence:** [ContractValueEvidence](crates/domains/chain/src/transaction/read.rs) retains both
original:Object and implementation_ref; [EVM projection](crates/domains/evm/src/transaction/read.rs)
supplies them; [the callback](crates/kernel/program/src/callback/native.rs) passes the admitted
original alongside decoded native evidence. The
[lifecycle report](crates/domains/chain/src/transaction/report.rs) retains that evidence.

**Counterexample:** a projection port providing only a decoded Receipt and receipt hash cannot
preserve this consumer's original bytes without another encoding, a resolver or a weakened output.
It also lacks the selected code reference needed by the retained evidence. An adapter cannot
independently derive a LeafDescriptor that includes its State's contracts.

**Resulting correction:** [RFC section 6.2](RFC_REFACTOR_SIMPLIFICATION.md#62-native-adapter-semantics)
now supplies the already-admitted Object, Receipt decoded from that same Object and factory-selected
intrinsic leaf ref. Receipt identity comes from Object.value_ref(). Retained evidence changes its
code-provenance field to leaf_ref in the eventual coherent schema cutover. No new context wrapper,
stored Observation, serializer retry or resolver is introduced.

**Limit:** this establishes why the previous proposed signature was incomplete. It does not prove
the corrected callback, target evidence schema or hot/cold execution has been implemented.

| Hypothesis | Execution status | Evidence conclusion | Available evidence / next missing result |
| --- | --- | --- | --- |
| H1: construction/erasure | Not run | Unvalidated | Need the compiling, changing-type specimen and deliberate rejection cases. |
| H2: owner-local extension | Not run | Unvalidated | Need independently reviewed common semantics and a second author's change evidence. |
| H3: canonical original/code provenance | Not run | Unvalidated | F1 corrected an insufficient port by source review; need literal-byte, codec and specialization probes. |
| H4: semantic Effect authority | Not run | Unvalidated | Need actual ID/ref lineage, scripted acknowledgement trace and any implicated physical evidence. |

### Execution baseline and predictions (recorded before implementation)

The actual checkout was clean at `a478fc341d993f3b9b747d72c53a6f027be41452`, the plan
commit: `git status --short` returned no entries and there were no subsequent commits. Current
production controls therefore use that exact revision. The owned disposable worktree is
`/tmp/mfm-simplification-specimen-a478fc34`, detached at the same revision. Main-checkout work
is restricted to this results record, necessary RFC corrections, and inert reproducibility evidence.

The specialist launches accepted construction/canonical and independent extension assignments of
`gpt-6.1-sol/high`, Effect/custody `gpt-6.1-sol/xhigh`, and independent read-only review
`gpt-6-astra/xhigh`. The coordinator's active model/effort cannot be independently introspected;
the requested `gpt-6-astra/xhigh` coordinator assignment is not claimed as verified. No worker
availability substitution was reported. Extension implementation starts only after the initial
construction specimen works and its ports are documented. The coordinator owns all shared records.

**Requirements traced.** Current Portfolio acknowledges finer source stages, but neither those
stages nor existing tests establish a product requirement for within-collection continuation.
Collection repetition remains a product uncertainty. Holdings mean exact asset/account/ledger,
raw unsigned base units, denomination, observation point and source coverage; scale alignment
supplies no price or FX fact. The distinct native fixture must not be forced into the scalar
lifecycle contract. Lifecycle requires its deployment, configuration and observation originals,
request/effective values and selected code provenance through Observe -> Validate -> Report.

The current Runtime trace is prepare/check -> native Read IO -> admit original -> fused
projection/interpretation -> append original/outcome. Cancellation before that append may repeat
Read work. Operational originals append before classification. Effects instead prepare/check ->
append command -> acknowledged adapter entry -> qualify settlement -> append settlement -> pure
interpretation. The Store response establishes caller acknowledgement; independent underlying
Store inspection establishes actual retained bytes. They are different facts. Interrupted physical
attempts are not necessarily recorded. Authority load/reservation and retained winning signed wire
prevent independent replacement after uncertain broadcast, subject to the actual identity lineage
review below. Current semantic recipes deterministically derive creation/call commands from
DeploymentRequest/DeployedContract, while current public stages use distinct EffectIds.

**Smallest proposed seam.** Program's `freeze`, document/StateData construction and executable
association are crate-private. A disposable `experiment_bridge` inside Program may construct
current StateData/NativeAbi descriptors and associate directly supplied callbacks after complete
qualification. It must not lower through AuthoringSource, Inject*, Plan or the source DSL. A typed
consuming builder and private factory erasure enter the actual immutable Program; production
Runtime, Journal, Store and canonical values remain unchanged. Existing wire evidence slots can
carry the native Receipt while ephemeral Observation remains inside the completion callback.
The selected bridge descriptor reference is a provenance stand-in in the existing
`implementation_ref` field, not implementation of the proposed `leaf_ref` schema. Target descriptor,
full target cold association and production deletion remain unvalidated through this seam.

**Predetermined discriminators.** These are predictions, not passes:

| Probe | Expected literal facts / independent oracle | Counterexample consequence |
| --- | --- | --- |
| Lifecycle construction | ConfiguredContract -> ObservedConfiguration -> ValidatedConfiguration -> ContractDeploymentReport; effective/observed scalar 42; target address twenty bytes of 4; getter selector `3fa4f245`; anchor number 7/hash thirty-two bytes of 7; exact supplied deployment/configuration/observation originals retained. Provider argument sink and underlying stored frames are separate from projection. | Missing output/provenance contradicts the chosen port. |
| Canonical normalization | Complete request and receipt fixture JSON is `{"value":7}`; pre-encoding Rust fields are 9. Selected decoding supplies 7 to checks, provider, projection and interpretation; output value 7. Each original serializer runs once. Compare literal bytes and stored Objects, not only hot/cold equality. | Using 9 after admission or re-encoding originals contradicts canonical authority. |
| Plain Observation | Lifecycle observation contains actual evidence plus `Cell<u8>`, with no Serde/MfmValue/Sync requirement; created and consumed within the pure fused callback. | A necessary serialized/Sync Observation contradicts the claimed bound removal. |
| Adjacency and pairing | Wrong ConfiguredContract -> Report adjacency and incompatible State/adapter Observation fail compilation at associated-type equality. Borrowed initial input remains available; Runtime independently rejects changed input. | Erased public composition or ignored mismatch contradicts typed construction. |
| Ownership | A shadow Rust owner with the same selected semantic key rejects before binder attachment and provider entry; both independent counters remain zero. | Silent replacement contradicts fresh ownership. |
| Local/native failure | Local binding disagreement has no provider call or operational append. A supplied native fault retains distinguishable nested causes and classification through cold inspection. | Internal disagreement cannot become authenticated native failure; discarded causes violate custody. |
| Extension | Independently authored equivalent holdings owner preserves raw 42/denomination 0 and empty 0 under reviewed native identity/point semantics. Its binding, receipt, fault and resource types differ. Compatible instances reuse one factory. | Central protocol branch or coerced units/rejection semantics refutes locality/sharing. |
| Specialization | Equal remaining stored contracts/family IDs plus behavior-changing hidden Observation parameters expose collision; distinct semantic revisions or checked instance facts separate meaning. | Same committed target identity with changed cold meaning is unsafe; an old-wire check alone cannot validate the target. |
| Effect schedule | Underlying command append completes while response is withheld: native/authority/signer/provider counters stay zero. After acknowledgement, retained authority determines exact winner. Settlement append is observed independently, response withheld, caller cancelled; reconstruction interprets retained settlement with no native IO. | Early IO/interpretation, replaced authority or re-signing acknowledged settlement refutes H4. |
| Partial outcome | After acknowledged deployment, maximum unsigned-256 plus one refuses at the existing checked-add domain boundary; successful deployment remains retained and configuration preparation count is zero. | Internal preparation failure or lost deployment exposure contradicts readiness ownership. |

The Effect worker will record exact old/new command/ref/key lineage and physical experiment selection
before implementing its fragment. Neither a historical managed pass nor a scripted Store response
establishes changed PostgreSQL authority mapping or real network behavior.

**Material uncertainties at prediction time.** The current-wire descriptor may prevent a small
callback association; validate by compiling the bridge and stop if a second execution engine is
needed. Collection interruption requirements are unspecified; retain them as unvalidated instead
of inferring product intent. The independent holdings owner may require different semantics;
review units/identity/trust/rejection first and use a distinct contract when necessary. Target
specialization identity is not supplied by the current wire. Authority lineage changes may require
managed physical evidence; select that evidence from actual keys/order before relying on controls.

### Execution result template

For each probe add one concise record under this section:

| Field | Record |
| --- | --- |
| Hypothesis and exact claim | H1-H4; identify the specific part exercised. |
| Candidate and control | Commit IDs/diff; current-wire bridge or target mechanism; relevant resource lifetime. |
| Expected facts and oracle | Predetermined values/bytes/identities; how the observer avoids the implementation under test. |
| Procedure and command | Actual probe/fixture, invocation, exit status and controlled interruption. |
| Observed evidence | Locations of compiler results, retained Objects/frames and reviewed native events. |
| Conclusion and consequence | Supported / Counterexample / Unvalidated; required correction or next design decision. |
| Remaining limits | Unexercised target wire, physical guarantees, native support or workload assumptions. |

Do not replace a missing run with an architect's agreement or an existing test's historical pass.

## 7. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| A bounded specimen can reach the actual callback boundary. | Current association/wire is coupled to superseded authoring machinery. | The prototype may become most of the cutover or hide defects behind a bridge. | Inspect dependencies and declare the seam before coding; limit claims and record obstruction. |
| Collection acknowledgement matches the product requirement. | Current stages/tests do not establish whether callers need per-source durability. | The chosen Read unit may lose required observable progress. | Trace Portfolio interruption requirements independently before accepting the boundary. |
| The second owner's semantics genuinely match a shared port. | Different native contracts can differ in units, identity, evidence or recovery. | Reuse may flatten meaning or require central special cases. | Review the semantic correspondence, then have an independent author implement it; use distinct contracts if needed. |
| Target identity remains sufficient without Observation schemas. | An old-wire harness may preserve a discriminator the target removes. | Cold association could select behaviorally different code. | Exercise the deliberate collision and actual target identity mechanism; otherwise retain an unvalidated conclusion. |
| Native authority controls remain reusable. | Stage removal changes EffectId/ref lineage and may alter custody ordering. | Recovery could load wrong authority or prepare another winner. | Compare actual lineage/key/order contracts and select focused physical evidence where changed. |
| Literal oracles can be observed independently. | Shared wrappers/projectors can repeat the same false claim. | A passing test could conceal canonical or acknowledgement divergence. | Inspect oracle construction, underlying Store bytes and native events separately. |

The fixture does not establish future monetary integrations, cross-run exclusivity, distributed
signing or workload capacity. Those claims require their own selected requirements and evidence.
