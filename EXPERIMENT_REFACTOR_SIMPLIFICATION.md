# Experiment: validate the refactor simplification architecture

**Status:** executed, independently reviewed and archived; exact-candidate CI and focused export passed; disposable specimen removed.
**Architecture baseline:** [RFC_REFACTOR_SIMPLIFICATION.md](RFC_REFACTOR_SIMPLIFICATION.md),
including its projection correction at `d72b2e08`.
**Available evidence:** source finding F1 and executed findings F2-F4, with per-claim limits in section 6.
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

| Claim | Conclusion | Evidence and boundary |
| --- | --- | --- |
| H1 direct typed construction/private erasure | **Supported** for the exercised mechanism | Real changing lifecycle, intended compiler rejection, exact initial commitment and preattachment owner checks through a direct current-wire bridge. Production source/ABI deletion remains unvalidated. |
| H2 independent native extension | **Supported** for the reviewed holdings fixture | Independent batch owner, one shared State, distinct native facts/resources/causes, no generic protocol branch; compatible bindings reuse one factory. Live protocol and product collection granularity remain unvalidated. |
| H3 canonical originals/provenance | **Supported** for canonical custody and selected experimental key | Literal request/receipt `7`, one encoding each, exact stored originals, non-Serde/non-Sync Observation, selected declaration reference retained. Production leaf_ref schema remains unvalidated. |
| H3 unconditional identity sufficiency | **Counterexample** | Same IDs/stored contracts but hidden specialization changes output `42` to `21` after cold Program load and fresh execution. This confirms the RFC's semantic-revision trust limit, not a contradiction of its qualified contract. |
| H4 semantic Effect/private native custody | **Supported** for the scripted schedule and focused PostgreSQL/Reth mapping | Actual DeploymentRequest/DeployedContract share one private routine; exact retained command/native ref/EffectId/winner and acknowledgement ordering; deployment exposure survives typed overflow. Remaining physical/cutover limits are explicit below. |
| Production target descriptor and provenance wire | **Unvalidated** | Current Program v9/NativeAbi and implementation_ref are explicit carriers; the complete target leaf_ref schema and cold-association cutover were not implemented. |
| Full physical failure/restart guarantees | **Unvalidated** beyond the named managed schedule | No real COMMIT packet loss, disk crash, signer process restart or new concurrent-winner stress campaign; preserved current controls do not establish those changed guarantees. |
| Production deletion, performance and collection interruption requirements | **Unvalidated** | No supported implementation was cut over or deleted, no workload benchmark was run, and current source stages do not settle product continuation requirements. |

### Execution baseline and predictions (recorded before implementation)

The actual checkout was clean at `a478fc341d993f3b9b747d72c53a6f027be41452`, the plan
commit: `git status --short` returned no entries and there were no subsequent commits. Current
production controls therefore use that exact revision. The owned disposable worktree was created at
`/tmp/mfm-simplification-specimen-a478fc34`, detached at the same revision, and removed after review. Main-checkout work
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

**Effect lineage prediction, recorded before its implementation.** Runtime EffectId v2 commits
RunId, Program reference, State/visit and exact command reference. Current reservation commands an
Eip1559TransactionCommand and keys authority by its reservation EffectId/native command ref.
Preparation commands ReservedEvmTransaction with another EffectId, but physical load/retain still
uses the reservation EffectId. Execution commands PreparedTransaction<R>; settlement carries that
third transaction EffectId. The candidate commands DeploymentRequest or DeployedContract directly;
one semantic EffectId supplies reserve/load/retain/settlement identity, while the recipe-derived
native command ref qualifies native authority. NonceDomain remains epoch/chain instance/sender.
The candidate predicts creation nonce 0 and configuration nonce 1 under a fresh fixture.

Current EvmTransactionSettlement lacks the target native-command-ref field, so a disposable native
receipt must retain that reference alongside the unchanged settlement. This is receipt evidence,
not another command/preparation wrapper. The mapping change requires an actual candidate managed
PostgreSQL/Reth experiment through the existing effect-e2e provisioning task, including retained
wire and node observations. Neither a historical managed pass nor a scripted Store response
establishes changed authority mapping or real network behavior.

**Material uncertainties at prediction time.** The current-wire descriptor may prevent a small
callback association; validate by compiling the bridge and stop if a second execution engine is
needed. Collection interruption requirements are unspecified; retain them as unvalidated instead
of inferring product intent. The independent holdings owner may require different semantics;
review units/identity/trust/rejection first and use a distinct contract when necessary. Target
specialization identity is not supplied by the current wire. Authority lineage changes may require
managed physical evidence; select that evidence from actual keys/order before relying on controls.

### Executed candidates, seam and reproducibility

All disposable source is preserved as the inert
[candidate patch](docs/experiments/refactor-simplification/candidate.patch.gz), against the clean
control `a478fc341d993f3b9b747d72c53a6f027be41452`. It is outside Cargo discovery in this checkout
and is not a supported API. The final candidate is
`32c5283cb969ad30310ba98d6a378b5a84d40fdd`, with no uncommitted source diff during final verification.
Its ordered logical commits are:

| Commit | Specimen content |
| --- | --- |
| `f966cb71` | Direct typed bridge, construction/canonical/compiler probes and coordinator-authored first holdings owner. |
| `a0f5f514` | Independently authored batch owner, resource isolation and specialization challenge. |
| `32c5283c` | Shared private Effect custody, scripted and managed observers, task selection and final formatting integration. |

Full hashes are in [candidate-commits.txt](docs/experiments/refactor-simplification/candidate-commits.txt).
Decompressed patch SHA-256: `5ee7baf167a141652f395871e33b93f81360e01b7cd74b23bd59f0a320749a79`.
The main-checkout prediction commits are `0c01b589` and `36a178f6`; both precede implementation.
The fixture's symbolic other-ledger/fixed-test-point was refined to typed BookLedger 19 and
BookPoint sequence 117 when the shared contract was agreed; raw `42`/`0`, denomination `0` and
liability exclusion stayed fixed. The first owner's `10^18`/denomination `18` oracle comes from
RFC section 16.2. No monetary conversion is inferred.

The bridge directly constructs current Program v9 declarations and calls Program::freeze. It never
calls AuthoringSource, Plan, Inject*, compile or source lowering. Existing deterministic EVM
translation/projection helpers supply native scalar meaning. Real Runtime, Journal, MemoryStore
and (in the managed probe) PostgreSQL execute and retain actual frames. A small experimental
intrinsic key includes mode, State/adapter IDs and stored contracts, excludes Observation and
is carried in current State/NativeAbi fields. The factory supplies that actual selected key;
existing consumer `implementation_ref` fields hold it as an explicit specimen stand-in.
This exercises the candidate key/claim mechanism, not the proposed production descriptor or
`leaf_ref` schema cutover. Stop/NoParams is the only supported recovery selection; unsupported
policy/checkpoint cases reject. No second scheduler, persistence engine or source DSL was built.

Reproduction commands and evidence boundaries are in
[reproduce.txt](docs/experiments/refactor-simplification/reproduce.txt). Reapply the patch in a new
worktree at the control commit, stage the added files for Nix's source view, and run the recorded
commands. The executable specimen was discarded after evidence extraction and independent review. Native public hashes,
addresses and authority epochs vary because managed services and the ephemeral signer are fresh;
relationships, literal command terms, quantity oracles and no-IO assertions remain reproducible.

### F2: construction, canonical custody and causal boundaries

The final focused run executes **seven construction tests**, plus the first-owner holdings test
and five independent extension tests. The
[Read probe log](docs/experiments/refactor-simplification/read-probes.log.gz) records their literal
observations. The two independent consuming compiler checks exit **101** for the intended
[E0271 adjacency mismatch](docs/experiments/refactor-simplification/compile-adjacency.log.gz) and
[E0277 Observation mismatch](docs/experiments/refactor-simplification/compile-observation.log.gz);
the rejection-checking shell gate exits 0. These are inspected diagnostics, not unrelated build
failures or assertions about incidental compiler prose.

The construction path is actual ConfiguredContract -> ObservedConfiguration ->
ValidatedConfiguration -> ContractDeploymentReport. Supplied predecessor originals are scripted
native scalar Objects with canonical bytes `"101"` and `"102"`; the Read specimen does not claim
to execute their earlier deployments. It retains those exact Objects and the complete getter
original, request/effective values and selected key. Actual provider arguments independently equal
target `[4;20]`, selector `3fa4f245`, anchor number 7/hash `[7;32]`, and the returned 32-byte ABI word
ends in `2a`. Effective and observed output are `42`. Exact output is independently decoded from
the underlying Store, including all three originals; cold re-association/inspection performs no
extra provider call. Changed initial effective value `43` rejects at Runtime before provider entry.

The normalizing request and receipt each start with Rust field `9`, encode once to the complete
literal `{"value":7}`, and are decoded from that admitted Object. Checks, provider arguments,
projection and interpretation all see `7`; output is canonical `"7"`. Exact retained intent and
receipt Objects extracted from the Store match the literal bytes and independently computed
literal digest. Both serializer counts remain one after cold inspection. The separate admitted
input remains `9`; no claim says every byte in the run is `7`. Observation has `Cell<u8>` and no
Serde/MfmValue/Sync requirement inside the fused pure callback.

Fresh conflicting Rust owners reject with attachment/provider counters zero. Local route mismatch
leaves only admission head 1 and provider count zero. The native operational fault retains exact
`get_scalar` -> cause code `731` / `fixture transport rejected getter`, Permanent classification
and cold original with one provider call. A refused original append separately retains Store cause
`932` and the attempted native original in invocation custody while underlying head stays 1;
there is no durable-audit claim through the failed Store.

The reviewer found ordinary prototype bugs in Read panic operation attribution, late handler
qualification and initially weak substring oracles. They were corrected and rerun: interpretation
panic remains ReadInterpret/Execute without persisted payload, and a malformed later handler
rejects before the first attachment. No validation was relaxed. See the
[construction record](docs/experiments/refactor-simplification/construction-results.txt) and exact
stored [normalization](docs/experiments/refactor-simplification/mfm-normal-frame.json),
[lifecycle](docs/experiments/refactor-simplification/mfm-lifecycle-frame.json) and
[fault](docs/experiments/refactor-simplification/mfm-fault-frame.json) frames.

### F3: independent extension and the specialization counterexample

The coordinator's first owner and shared holdings State worked before extension authorization;
[handoff hashes](docs/experiments/refactor-simplification/extension-handoff.txt) record that boundary.
The independent worker changed only `tests/simplification/extension.rs` and its consuming test.
Shared holdings, bridge, Runtime, Journal, Store and the first owner acquired no extension branch.
One first-owner factory serves route-a/route-b. One second-owner factory adds a distinct BookBinding,
BookReceipt enum, nested BookFault/BookSource and Arc<BookResource>. Its native module imports the
shared holding types and kernel ports, not EVM native types. Full imports/change-site accounting is
in the [extension record](docs/experiments/refactor-simplification/extension-results.txt).

The first owner preserves raw `1000000000000000000`, denomination `18`, point 7. The second owner
returns batch lines in native order empty/retained, while requested/output order is retained/empty,
with exact raw `42`/`0`, denominations `0`/`0`, ledger 19 and point `{"sequence":117}`. Its entire
batch original remains exact in output and underlying Store. These owners share fungible quantity
meaning with explicit native identity and observation policy, not identical anchoring trust or a
global snapshot. Cold inspection adds zero native calls.

Missing requested account (2101), duplicate account (2102), native liability (2301), wrong book
(2201) and wrong point (2202) each make one instrumented invocation and retain distinct exact nested
Permanent operational originals. Missing does not become zero; duplicate data cannot overwrite a
prior account. Caller liability/local route mismatch rejects with zero IO and admission-only head.
A mixed Catalog rejects the wrong resource table before attachment; the selected Book owner runs
with the unselected scalar owner's table absent. Stored success/fault frames are archived alongside
the logs; these are scripted-native evidence, not live protocol certification.

The specialization probe holds **all remaining stored types and family IDs equal** while changing
Observation-only const specialization and arithmetic from division by 1 to division by 2. Keys
are equal because no Observation schema is hidden in the bridge NativeAbi. Fresh installation
rejects conflicting owners before attachment. A sole replacement under unchanged IDs loads exact
old Program bytes and a **new fresh RunId** executes `21` instead of `42`, retaining the same native
original/key. This is not retained-run resume or re-interpretation of its acknowledged result.
Changing the adapter revision to @2 rejects the old document before attachment. This is a concrete
counterexample to unconditional identity sufficiency; it supports retaining the RFC's explicit
revision trust and denies automatic executable authenticity. No new ObservationId trait is justified.

### F4: one semantic Effect, private authority and partial exposure

The actual DeploymentRequest and DeployedContract command types use two typed adapter pairings
and **one private custody routine** calling existing reserve/prepare/execute helpers. The old
control is executed too: three distinct public EffectIds exist, while physical load/retain keys
use the reservation ID. In the candidate, one semantic EffectId is simultaneously the stored
command ID, reservation/load/retain key, native settlement ID and output deployment ID. The
semantic command ref deliberately differs from the recipe-derived native command ref; the latter
matches retained native authority and the native settlement original. Epoch/domain and first-winner
SQL rules are unchanged, but the changed ID mapping was tested physically.

The scripted Store wrapper first awaits the real underlying append, then withholds its response.
An observer bypasses it. At committed command head 2, adapter/authority/signer/provider counters
are zero. Releasing acknowledgement allows reservation, signing, winner retention and native IO.
At committed settlement head 3, interpretation remains zero; cancellation and reconstruction
resume through real Runtime retained-fact qualification. Interpretation preserves exact original,
selected key, semantic/native refs and EffectId with no further native IO. A separate overflow
case runs acknowledged deployment then max-U256 plus one: typed AdditionOverflow retains the
successful DeployedContract and original hash; configuration preparation is never entered.

The managed candidate uses real PostgreSQL authority/Store, the actual keystore and Reth. An
independent backend observes committed rows while responses are withheld, and independent authority
loads and node calls observe retained wire/receipts/value. Cancellation after actual broadcast
reuses the same command and winning bytes without re-signing. Creation and configuration reserve
nonces 0/1 and use two signatures in total. Creation input equals the pinned 497-byte artifact,
zero value/gas 2,000,000/fees 1,000,000,000 and 10,000,000,000; configuration targets the admitted
created address with zero value/gas 200,000 and exact calldata
`1eb25e0a000000000000000000000000000000000000000000000000000000000000002a`.
Independent getter `3fa4f245` returns the full word for `42`. Submitted bytes equal the retained
winner; acknowledged settlement interpretation and terminal replay leave native counters unchanged.
Repeated physical submissions using that same retained authority and wire are permitted; this is
not an exactly-once physical-call claim.

The independently reviewed complete temporary frames and raw creation bytes contain compiled
solc output. Repository policy keeps that artifact temporary. The committed
[scripted facts](docs/experiments/refactor-simplification/scripted-observed.json) and
[managed facts](docs/experiments/refactor-simplification/managed-observed.json) preserve the complete
native receipt originals, public authority/command/code refs, independent node evidence and
creation-wire digest/length, **not a lossless copy of the complete temporary frames**.
The [extractor](docs/experiments/refactor-simplification/extract-effect-evidence.py) documents that
subset; pinned solc and the archived specimen reproduce the omitted compiled bytes.

This is physical evidence for the named identity correspondence and retain-before-submit schedule.
Withheld responses are injected by wrappers; no real COMMIT packet loss, disk crash, production
finality or signer process restart was simulated. The same keystore owner survives reconstruction.
Concurrent first-winner contention was not newly stress-tested. Existing public reserved/prepared
Rust types are used as private helper temporaries in this specimen; their public APIs were not
privatized or deleted, so that eventual cutover remains unvalidated.

### Verification, review and accounting

The final exact-candidate command ledger is
[commands.txt](docs/experiments/refactor-simplification/commands.txt). All Rust tools ran in the
default Nix shell. Focused Read/extension execution passed 13 tests; both negative compiler checks
rejected as intended; formatting passed. Exact-candidate `nix run .#ci` **passed, exit 0**, with
nine tasks and zero failures in 703.46 seconds, run `run-bd2cb65d99b5e2af8a45b27e294c8dd0`.
The [final CI log](docs/experiments/refactor-simplification/ci.log.gz) includes explicit candidate
scripted and managed Effect execution, not merely ignored test discovery or historical controls.

Two earlier attempts on the unchanged candidate exited 30 and 38 due to disk exhaustion during
compilation; both [first](docs/experiments/refactor-simplification/ci-enospc.log.gz) and
[second](docs/experiments/refactor-simplification/ci-enospc-2.log.gz) logs are retained. Recovery removed
only the owned disposable focused cache with pinned Cargo clean, then moved its static-library
verification cache into owned RAM storage with per-file SHA-256 checks. The candidate source and
user checkout cache were unchanged. The successful third run follows those infrastructure failures.

The managed task sets TMPDIR to its state directory and removes that directory on exit. Initial
export therefore failed with FileNotFoundError, despite successful assertions and retained digests.
The [capture wrapper](docs/experiments/refactor-simplification/capture-effect-evidence.py) reruns only
the focused Effect task on the unchanged candidate, under fresh owned state, and links only its dump
output to a persistent temporary directory. Two setup launches refused group-writable output roots;
mode 0700 corrected the wrapper without weakening framework ownership checks. The archive records
those refusals. The focused export **passed, exit 0**, in 221.36 seconds,
run `run-0041e303a87baf0fb6f8056dbf1720b4`; all 17 raw trace files matched their lengths and
SHA-256 digests. Its [log](docs/experiments/refactor-simplification/effect-export.log.gz) identifies
the final archived Objects; the public extractor also exited 0. This archival correction changes
neither physical authority nor the independent observers. It does not substitute earlier candidate
results or dumps. Fresh signer/service values differ between CI and the export run as expected.

The independent reviewer remained read-only, inspecting source, exact compiler diagnostics,
fixture premises, retained frame Objects, native traces and the bypass-observer construction.
Its final record is [independent-review.txt](docs/experiments/refactor-simplification/independent-review.txt).
No architect agreement is counted as executable evidence. Ordinary prototype defects were fixed;
no required behavior contradicted the qualified RFC boundary in a way requiring a broader target
redesign. The RFC needs an execution-status correction and the concrete specialization finding;
its current semantic-revision trust contract already covers that counterexample.

| Accounting category | Added / deleted / net physical source lines |
| --- | --- |
| Disposable Program bridge and export | 971 / 0 / +971 |
| Disposable private native custody helper and export | 85 / 0 / +85 |
| Disposable consuming tests, native fixtures and compiler harness | 3,700 / 0 / +3,700 |
| Disposable task selection | 4 / 0 / +4 |
| Total specimen relative to a478fc34 | 4,760 / 0 / +4,760 |
| Archival capture/extraction scripts (outside specimen total) | 162 / 0 / +162 |
| Resulting supported production implementation | **0 / 0 / 0** |

[Numstat](docs/experiments/refactor-simplification/specimen-numstat.txt) defines this reproducible
physical-line count, including comments/blank lines; it is not a claim of eventual refactor savings.
Necessary specimen complexity is typed factory/owner qualification, two mode-specific callback
paths with canonical/error containment, the native receipt's missing command-ref fact, explicit
native resources and independent observers. Shared holdings has two actual owner factories;
compatible bindings need no extra factory, while two different semantic Effect command types need
two pairings sharing one custody routine. No dependencies, manifests, Runtime/Journal/Store/SQL
algorithms or production schemas changed. The bridge grew beyond its initial estimate to preserve
both modes, panic provenance, owner/handler qualification and current-wire restoration.

What simplified in the exercised path: ordinary typed appends replace source traversal/injection;
Observation has no stored mirror; one acknowledged semantic Effect replaces three public stages
while retaining private nonce/wire durability. What was physically removed from production: **none**.
All executable specimen source was removed with its owned worktree after extraction and review;
the 682-file owned RAM cache and 17 full temporary Effect dumps were also removed. The patch is
an inert reproduction artifact. Net production savings, performance and complete superseded API
removal require the coherent production cutover and are not inferred from this experiment.

## 7. Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation / decision |
| --- | --- | --- | --- |
| Production target descriptor/association preserves the exercised ports and identity. | Execution used an experimental key inside Program v9/NativeAbi and old provenance fields. | A full cutover could lose owner, schema or cold-restoration guarantees. | Implement the complete target wire/evidence cutover and retained-run cold acceptance before calling it supported. |
| Whole-collection acknowledgement is the product requirement. | Existing Portfolio source stages/tests establish behavior, not independent product intent. | Repeating an unfinished collection could lose required source-level progress. | Obtain consuming interruption requirements; test completed/unfinished collection recovery against them. |
| Real native owners can satisfy the reviewed holdings contract. | Both holdings providers are scripted and have explicitly different observation policies. | Local extensibility could be mistaken for native truth, anchored interoperability or global coherence. | Test each supported provider's actual identity/anchor/coverage/rejection semantics; use distinct contracts for different meaning. |
| Authors revise every behavior-changing semantic identity. | The executed unchanged-ID specialization replacement is accepted and changes output. | Cold code can silently change meaning despite identical stored key and fresh-owner checks. | Enforce reviewed revision changes and missing-revision rejection; executable-byte authenticity requires a separate attestation design. |
| Broader target builder/resource behavior follows the small specimen. | Stop-only policy, limited root/owner probes and one selected owner at a time omit full checkpoints/custom policies/resource-free historical inspection. | Successful ports could conceal missing full construction/inspection guarantees. | Retain RFC section 16.2 acceptance; no policy-instance, foreign-checkpoint or full resource-free inspection claim here. |
| Full physical custody behavior survives the future cutover. | Focused real PostgreSQL/Reth mapping passed, but wrapper acknowledgement withholding is not a crash/network-fault campaign; old physical helper structs remain. | Concurrency, real COMMIT ambiguity, signing restart or deleted helper contracts could invalidate broader safety. | Preserve current ambiguity and same-owner signer scope; execute selected race/failure/restart acceptance when those guarantees change. |
| Performance, workload shape and net production simplification meet product needs. | No throughput/latency/projection-cost benchmark or production deletion was performed; specimen adds 4,760 temporary lines. | Local extension success could overstate speed or eventual code savings. | Measure named workloads and account for added/deleted production code during complete cutovers. |

The bounded seam itself was executable; it is no longer an unresolved feasibility question for the
named probes. The ZEC -> Ethereum WBTC -> Aave route, cross-run exclusivity, distributed signing,
production finality and real monetary integration remain outside scope. The evidence supports
continuing with the named architectural mechanisms, not approving the whole RFC or starting those
integrations.
