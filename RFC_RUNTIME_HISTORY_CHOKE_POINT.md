# RFC: Runtime History Choke Point

Status: ready for implementation planning; implementation has not started

Scope: structured operation authoring and expansion, state execution kinds, typed failure
handling, run-history ownership, deterministic scheduling, external-access recording, effect
ambiguity, cross-run resource authority, EVM nonce ownership, adapters, transports, stores,
replay, telemetry boundaries, and deletion of the arbitrary state graph and generic executor

Compatibility: this is a breaking target design. The current code, `docs/design.md`, and
`docs/architecture.md` continue to describe the implemented graph/executor design until one
complete cutover deliberately replaces that contract. This RFC defines no compatibility reader,
dual writer, graph-to-sequence adapter, fallback executor, or parallel execution authority.

## Executive Decision

An MFM operation is a declaration-ordered, typed, structured program.

The operation itself is the sequence. Its author-visible structural forms are:

```text
State
Match
FanOut
```

Only `State` is an executable occurrence. `Match` and `FanOut` describe structured control around
state occurrences. An authored child `OperationCall` is composition sugar that expansion removes,
not a fourth control form or certified occurrence. Every lexical block has one typed result
expression, and the operation root's result is:

```text
OperationOutcome<Output, Failure> =
    Success(Output)
  | Failure(Failure)
```

An outcome is not a binding, instruction, node, or separately scheduled cursor position.
Authoring helpers such as `succeed(value)` and `fail(failure)` only construct the root result
expression. Internal branch, fragment, handler, and lane results are likewise typed lexical
values, not control instructions.

`Match` performs exhaustive conditional control over an ordinary history-bound value encoded with
the kernel's closed tagged-sum contract; a successful state output may supply that value. The
private `StateOutcome` discriminator itself is not a public `Match` input: the structured state
binding exposes success as its output and routes a typed failure into its one exact structural
failure continuation. That failure continuation's normally completing path reaches the designated
handler. `FanOut` is the only place where more than one state may be eligible at once. Its lanes
are bounded `Pure` and `Read` blocks with collect-all semantics, and the initial certified contract
allows nesting to depth two so the existing portfolio network fan-out may retain each child EVM
operation's read fan-out.

An arbitrary DAG is not an execution contract. A graph may be derived for diagnostics or
visualization, but it is never admitted as scheduling authority.

Operations continue to expand purely:

```text
AuthoredProgram
    -> deterministic pure expansion
ExpandedProgram
    -> certification
CertifiedProgram
    -> admission and execution
```

Expansion may inject typed states and structured control for domain requirements, security,
provenance, failure routing, resource acquisition, and durable audit facts. Expansion is
call-site-local, finite, deterministic, bounded, and visible in the admitted program. It is not an
ambient runtime hook or an unrestricted AST-rewriting plugin system.

Only `CertifiedProgram` is execution authority. Runtime does not know whether a state was authored
directly, inlined from a child operation, injected by an EVM capability expansion, or wrapped by a
framework policy.

Runtime is the sole active interpreter and run-history mutation owner. This is not a deployment
singleton. Multiple processes may assemble workers against the same qualified authoritative
lineage, but exact-head compare-and-append and store fencing serialize their durable actions.

Outside `FanOut`, a run has exactly one current state occurrence. Inside `FanOut`, it has one
cursor per declared lane. Runtime executes only the state occurrence named by the verified cursor,
including a lane's current state; a lane or fan-out group is never executable. Runtime does not
scan a global ready set, rank nodes, spread authorizations, or infer control flow from value
availability.

Certified states retain three semantic execution kinds:

```text
Pure
Read
Effect
```

`Effect` means that the state may mutate an external target or consume an exclusive external
capability. Effects are forbidden inside the initial `FanOut` contract.

Fallibility is a static contract property, not an inference from execution kind or implementation
code. One explicit certified sum owns it:

```text
FailureContract =
    Never
  | Typed(RetainedValueContract)
```

`Never` resolves only to `KERNEL_NEVER_FAILURE_CONTRACT_REF` and has no retained-value contract,
codec, producer slot, or canonical value. Every `Typed` variant is fallible. The same rule applies
to a fragment boundary. Every fallible state occurrence and fragment boundary has exactly one
exhaustive failure continuation. A lexical scope may register a finite table of exact source
failure contracts to ordinary typed failure-mapper states; `.or_default()` is legal only when
exactly one entry matches. A scope whose failure contract is `Never` has no default and requires
explicit total recovery.
A wrapper may carry the protected failure only through its exact affine fragment boundary to that
one call-site handler; the protected failure's normal-control continuation cannot insert a second
handler or directly bypass the first. A distinct failure newly committed by an intervening
support state follows its own exact plan and may causally supersede the protected failure. The
default handler is `Pure`, infallible, consumes the exact `FailurePlanBound` typed failure, and
maps it into that operation or fan-out lane's declared failure contract. Reads, effects, retries,
fallbacks, and compensations are explicit states in explicit branches; they are never hidden
inside the default handler.

`Read` and `Effect` use one private Runtime-owned access bracket:

```text
Prepared<K>
  -> ExternalAccessAuthorized committed
  -> Authorized<K>
  -> exactly one registered-invoker entry
  -> PendingObservation<K>
  -> ExternalAccessObserved committed
  -> CommittedObservation<K>
       | Returned | SafeFailure
       |   -> state settlement
       |       | ProposedStateOutcome
       |       |   -> exact-head transition append
       |       |       -> StateTransitionCommitted
       |       |       -> committed StateOutcome
       |       | InvalidEvidence
       |           -> blocked; no semantic transition
       | SupersededBeforeEntry                    [Effect only]
       |   -> Refreshable next physical attempt
       | EntryUnknown                             [Effect only]
       |   -> parked; no semantic transition
       | IntegrityFault
           -> blocked; no semantic transition
```

where `K` is `Read` or `Effect`.

At most one unresolved authorization exists for one current occurrence. A second effect
authorization cannot be constructed while the first attempt may have entered. Process loss after
authorization therefore leaves that run parked at the current effect with no legal successor in
this RFC. A future same-occurrence reconciliation protocol would be required; declaration order
cannot prove whether an external target was entered.

Ordinary definite failures do not park runs. Every definite state-facing completion that the
product expects to handle is either a reviewed typed response or a reviewed redaction-safe
`SafeFailure`. The state deterministically interprets every valid expected instance into success
or typed failure; malformed or inconsistent evidence produces `InvalidEvidence`. A typed failure
follows the operation's explicit or default failure path, which may recover or close the current
run as failed. A later invocation receives a different `run_id` and is not blocked by the earlier
run's history status. Independent cross-run resource invariants still apply.

Integrity violations, unavailable run-history persistence, and possible-entry ambiguity are not
ordinary failures. They cannot be converted into state failure merely to obtain liveness.

The generic executor is removed. It must not remain as a second interpreter, a wallet driver, a
retry engine, or an adapter with a renamed lifecycle. Meaningful EVM progression is injected as an
explicit structured program of ordinary `Pure`, `Read`, and `Effect` states.

Cross-run uniqueness remains outside one run's Runtime. An EVM sender uses one narrow durable
wallet-nonce resource authority. It owns only the atomic nonce and writer-fencing invariants.
It does not fold run history, choose transaction steps, query an EVM provider, schedule retries,
or decide terminal run meaning.

The resulting shape is:

```text
operation DSL
    |
    v
pure expansion --> certification --> CertifiedProgram
                                      |
                                      v
                      Runtime -- sole writer --> RunHistory store
                         |
                         +-- authorized Read ----> registered invoker --> transport/provider/store scanner
                         |
                         +-- authorized Effect --> registered invoker --> transport/provider/signer
                         |
                         +-- authorized Effect --> WalletNonce authority
```

PostgreSQL's client and wire protocol are infrastructure transports. A schema-aware PostgreSQL
implementation that owns transactions, compare-and-append, fencing, exact idempotency, or nonce
uniqueness is an authority-bearing adapter/store. Calling PostgreSQL a transport must not hide
those responsibilities or expose a raw pool.

## Why The Existing Design Is Wrong

### Physical persistence was centralized, but the recording obligation was not

The store can be the only component that physically appends run records while a caller still owns
the obligation to remember the second half of an external-access protocol:

```text
caller
  -> append authorization
  -> invoke live boundary
  -> inspect or encode result
  -> append observation
```

An ordinary return between invocation and observation can silently violate the audit contract.
One private component must own authorization, affine invocation authority, totalization, pending
observation material, and observation commit as one no-normal-escape protocol. That owner is
Runtime.

### The executor became a second Runtime

The current generic executor owns or participates in another history, fold, current view,
resource allocation, target-attempt authorization, delivery planning, restart loops, retained
evidence, terminalization, and writer fencing.

Those are interpreter responsibilities. Nesting that lifecycle below Runtime duplicates the
failure protocol:

```text
Runtime authorization
  -> executor ensure
       -> executor authorization
       -> target call
       -> executor observation
       -> executor terminalization
  -> Runtime observation
  -> state settlement
```

Removing the executor does not remove the real work. It gives each responsibility one owner:

- operations and expanders own deterministic structured composition;
- states own reusable domain request and outcome semantics;
- Runtime interprets one certified cursor and orchestrates run-history mutation;
- the store owns legal append, exact-head CAS, and the callback-free run fold;
- adapters execute one already-selected operation;
- transports perform bounded protocol exchanges; and
- narrow resource authorities own cross-run invariants.

### The arbitrary DAG obscures the intended control model

The product's intended default is declaration order. Conditional control exists to branch on
typed state outcomes, and parallelism exists only when explicitly declared.

The current authored representation instead stores unordered node and binding bags, canonicalizes
nodes independently of declaration order, infers readiness from alternative producer groups, and
globally ranks eligible actions. That creates cycle, reachability, dependency-skip,
required-success, all-nodes-terminal, and fairness machinery that is not required by the product's
control model.

It also makes fan-out indirect. A collect-all operation should declare an ordered fan-out and
receive an ordered collection, rather than represent several results as alternative sources for
one destination.

The replacement is a structured program whose declaration order is semantic. A derived graph is
permitted only as a view.

### A state output cannot itself be a cross-run lock

An output such as `ReservedWalletNonce` is necessary but not sufficient. A run journal orders one
run; two runs can otherwise choose the same nonce concurrently.

The output is proof of a durable reservation, not the exclusion primitive. The wallet-nonce
authority's atomic transaction creates cross-run exclusion. The producer-bound typed value
provides within-run causality and provenance.

### Arbitrary wrappers would recreate hidden control

Pure expansion is valuable only while the expanded behavior is ordinary certified structure.
Allowing a wrapper to duplicate its protected effect, reorder siblings, reach across lexical
scopes, inspect live state, introduce an unbounded loop, or execute ambient logging would recreate
the graph/runtime indirection under another name.

Expansion is therefore a typed, finite substitution mechanism. Its power comes from composing
ordinary states and explicit control, not from bypassing them.

## Goals

- Make declaration order the default and authoritative operation execution order.
- Replace the arbitrary execution DAG with one structured program of declaration-ordered `State`
  bindings, `Match`, `FanOut`, and one typed root outcome.
- Preserve pure deterministic operation and child-operation expansion.
- Allow typed pre-, success-post-, failure-post-, and domain-requirement state injection.
- Make expansion finite, bounded, content-addressed, reproducible, and visible at admission.
- Require one exhaustive typed failure continuation for every fallible state occurrence or
  fragment boundary.
- Provide lexical operation- and lane-scoped default `Pure + Never` failure-handler states.
- Keep ordinary definite failures recoverable or terminal rather than indefinitely parked.
- Restrict initial fan-out to bounded, declaration-ordered, collect-all `Pure` and `Read` lanes
  with certified nesting depth two.
- Make Runtime the interpreter of one event-sourced structured state machine: the append-only fold
  derives its current cursor, while Runtime executes only the state named by that cursor. It is
  not a mutable in-process machine or a global graph scheduler.
- Make every normal return after authorized access structurally pass through observation commit.
- Preserve `Pure | Read | Effect` as certified semantic state kinds.
- Keep effect-bearing states visible for audit, security analysis, and recovery.
- Make store validation the authoritative enforcement boundary for persisted history.
- Preserve five append-only run-history record families and exact-head atomic append.
- Keep runtime and store generic with respect to EVM, nonce, expansion origin, and handlers.
- Inject EVM nonce and transaction prerequisites through registered semantic expansion.
- Replace generic executor resource machinery with the smallest resource-specific authority.
- Make normal resource-fence rotation a proven-pre-entry physical continuation rather than a
  semantic failure or integrity poison.
- Keep state logic deterministic and free of ambient IO.
- Separate semantic audit facts from best-effort operational telemetry.
- Keep secrets and provider-controlled diagnostics out of persisted and public surfaces.
- Delete superseded graph and executor concepts rather than preserve compatibility paths.

## Non-Goals

- Supporting arbitrary DAGs, jumps, loops, overlapping joins, or first-available-source control.
- Making declaration order a mere scheduler tie-breaker while retaining graph readiness.
- Allowing effects inside the initial `FanOut` contract.
- Defining fail-fast cancellation, detached fan-out lanes, or completion-order result semantics.
- Adding open-ended retry, polling, backoff, failover, or circuit-breaker policy.
- Automatically re-entering an effect after possible external entry.
- Treating a process lease or Runtime cursor as a target-enforced resource fence.
- Making authorization, external IO, and observation one atomic database transaction.
- Guaranteeing an observation after process, machine, or storage loss.
- Preserving every returned read or effect through a universal outbox.
- Making Runtime a cross-run resource coordinator.
- Allowing wrappers, state callbacks, adapters, transports, or applications to inspect or append
  run history.
- Allowing expansion to inspect a live adapter or transport to choose topology.
- Making best-effort logging or telemetry availability part of run correctness.
- Exposing PostgreSQL pools or generic query authority to Runtime or live adapters.
- Persisting credentials, private keys, signatures, raw signed envelopes, provider bodies, URLs,
  or unreviewed error strings.
- Encoding authorization and observation as fake semantic transitions.
- Letting an implementation plan weaken or reopen the frozen RFC contract without first amending
  this RFC.

## Terminology

### Structured program

The declaration-ordered authored and expanded forms have one normal value channel and one lexical
failure channel:

```text
AuthoredProgram<Output, Failure> =
    AuthoredFailureScope<Output, Failure> {
        failure_contract: FailureContract<Failure>,
        lexical_failure_mappings:
            match failure_contract {
                Never => EmptyExactMap,
                Typed(ScopeFailureContract) =>
                    ExactFiniteMap<
                        SourceFailureContract,
                        PureNeverMapper<
                            SourceFailure,
                            DefaultScopeFailureRoute<
                                Failure,
                                ScopeFailureContract,
                            >,
                        >,
                    >,
            },
        body:
            AuthoredOrderedBlock<
                Output,
                Failure,
                OwnsFailureScope,
            >,
    }

AuthoredOrderedBlock<Success, ScopeFailure, ScopeOwnership> {
    failure_scope:
        OwnsFailureScope
      | InheritsFailureScope(ExactLexicalScopeToken),
    declarations: [
        AuthoredStateCall
      | AuthoredOperationCall
      | AuthoredMatch
      | AuthoredFanOut
    ],
    tail:
        AuthoredBlockTail<Success, ScopeFailure>,
}

AuthoredBlockTail<Success, ScopeFailure> =
    Normal(TypedExpression<Success>)
  | ScopeFailure(TypedExpression<ScopeFailure>)

OperationOutcome<Output, Failure> =
    Success(Output)
  | Failure(Failure)

LaneOutcome<Output, Failure> =
    Success(Output)
  | Failure(Failure)
```

`Normal` and `ScopeFailure` above describe the two structural tail channels of a lexical
block. They are not bindings, instructions, scheduled occurrences, or author-visible `Return` and
`Fail` forms. A normal branch arm produces the `Match` binding's declared value. A failure arm
exits only its current lexical failure channel. There is no non-local successful completion:
`ScopeResult`, `ArmResult`, `RecoveryResult`, and general early-enclosing-success paths are not
part of the initial contract.

The `Output`, `Failure`, and other payload positions denote typed lexical expressions that
reference admission roots or dominating producer locals; they are not planning-time domain
values. A block's normal result is its statically declared tail expression. A fallible binding's
sealed continuation may recover that binding's output and continue, or produce the exact current
scope failure. At the operation root, the fold nominally wraps the selected channel as
`OperationOutcome`; at a lane boundary it nominally wraps it as `LaneOutcome`. Neither wrapper is
an executable control form.

`OperationCall` exists only in `AuthoredProgram`. Expansion replaces it with one internal
`FragmentBinding` whose boundary has the same typed success and failure contracts. Authored
`Match` arms and `FanOut` lanes recursively contain ordered blocks. Only operation, fragment, and
lane constructors mint `OwnsFailureScope` and own exact failure maps. Match arms and handler-route
blocks carry the unforgeable `InheritsFailureScope` token and cannot install or shadow a map.
Sequence is implicit in ordered bindings. There is no public `Sequence` graph node, jump,
non-local return, or general bytecode.

### Authored, expanded, and certified program

- `AuthoredProgram`: the canonical result of pure operation authoring. It may contain typed
  `OperationCall` declarations.
- `ExpandedProgram`: the canonical structured program after semantic capability requirements,
  recursive child-operation substitution, framework policies, and failure completion inject
  their ordinary states. It contains no `OperationCall`.
- `CertifiedProgram`: the validated, content-addressed execution authority admitted for a run.

There is one canonical certification root:

```text
CertifiedProgramComponents {
    certified_program_contract_ref,
    entry_point_contract_ref,
    qualified_entry_point_admission_policy_ref,
    authored_program_ref,
    expanded_program_ref,
    expansion_profile_ref,
    expansion_proof_ref,
    policy_coverage_proof_ref,
    public_input_output_failure_contract_refs,
    certified_structural_bounds,
    state_capability_adapter_signer_resource_manifest_closure_ref,
    secret_free_implementation_manifest_closure_ref,
    certification_predicate_set_ref,
}

CertifiedProgram {
    components: CertifiedProgramComponents,
    canonical_component_closure_digest,
}

canonical_component_closure =
    discover from every ContentRef encoded in canonical
    CertifiedProgramComponents field order:
      walk(expected_object_type_domain_tag, exact_content_ref):
        - reject exact_content_ref if it was previously encountered
          under a different object-type domain tag;
        - reject the pair if it is already on the active traversal stack;
        - if the pair is completed, return without emitting it again;
        - resolve the exact canonical object bytes and require the
          expected registered object-type domain tag;
        - add the pair to the active stack and immediately emit
          (
              expected_object_type_domain_tag,
              exact_content_ref,
              exact_canonical_object_bytes,
          );
        - enumerate outbound ContentRefs from registered reference
          positions in schema field order, using declaration order for
          sequences and canonical-key order for maps, and recursively
          walk each pair in that order;
        - remove the pair from the active stack and mark it completed;
        - reject an absent object, a content-reference mismatch,
          trailing/unparsed bytes, or an outbound reference in an
          unregistered position.

canonical_component_closure_digest =
    domain_separated_hash(
        "mfm.certified-program-closure.v1",
        canonical bytes of CertifiedProgramComponents,
        canonical_component_closure,
    )

CertifiedProgramRef =
    content_ref(
        domain_separated_canonical_bytes(
            "mfm.certified-program.v1",
            CertifiedProgram,
        ),
)
```

Each closure entry carries its exact already-canonical object bytes as raw JSON inside the enclosing
canonical preimage. Implementations MUST NOT serialize those bytes as a JSON numeric array; that
representation preserves the bytes but can expand a valid qualified program past the generated
canonical-document budget. The raw JSON form remains deterministic because the object bytes have
already passed canonical validation before insertion.

`CertifiedProgram`, `CertifiedProgramRef`, and `canonical_component_closure_digest` are not
closure inputs and can never be reached as outbound component references. The digest preimage is
therefore non-self-referential. Registered schema field order plus the stated sequence/map rules,
active-stack cycle rejection, and first-visit deduplication make one canonical closure for a DAG,
independent of object-store enumeration or map implementation.

The expansion and policy-coverage proofs are components of this object closure; there is no
separately admitted “certificate” that can authorize a different expanded program, profile,
proof, manifest, or implementation closure. Admission retains `CertifiedProgramRef` and its
complete content-addressed object closure. Any repeated authored, expanded, profile, proof, or
manifest reference in `RunAdmitted` is an audit projection that must equal the exact value inside
that root—it is never independent authority. The process-private qualified registry must exactly
satisfy the root's secret-free implementation manifest closure before admission.

The root does not choose its own certification rules. The process-qualified entry-point registry
resolves the admitted entry-point identity to one immutable, secret-free admission-policy object
that binds the exact `entry_point_contract_ref`, `certified_program_contract_ref`,
`certification_predicate_set_ref`, required `expansion_profile_ref`, exact policy versions, and
coverage obligations. Its content reference must equal
`qualified_entry_point_admission_policy_ref`. Certification evaluates the exact bound predicate
set, and the store's purpose-limited admission verifier revalidates that binding, re-evaluates
every bound certification predicate over the complete closure, and verifies every included proof
against the qualified policy before accepting `RunAdmitted`. A caller-selected policy or
predicate set, a valid proof under a different set, and a stale or unqualified policy object are
rejected. This verifier has no implementation-invocation, history-query, resource, or writer
authority. Recorded verification consumes the same immutable public policy object and
predicate-set semantics; it never trusts a root-selected verifier.

### Structural path and identities

Every occurrence has:

- a `SemanticCallId`, derived from the complete authored call-instance path plus the stable local
  label and preserved for a protected call through wrapping; and
- an `OccurrenceId`, derived from the complete normalized structural path, including expansion
  policy identity, local expansion label, branch arm, fan-out group, and lane ordinal.

Lexical order determines execution. Stable labels anchor semantic identity. Inserting one earlier
step changes the program hash and order but need not rename every later semantic call.
Labels are unique within their lexical authoring or expansion scope. A state introduced by
expansion receives a semantic subcall identity derived from the protected call and the policy's
unique local path.
Stable `Match` arm labels and `FanOut` lane keys extend the authored call-instance path for calls
inside those scopes. Declaration and lane ordinals determine order, but are never substitutes for
those stable identity keys.

### Implementation and physical binding

An immutable implementation binding selects the certified state/capability contract and registered
invoker code. A secret-free physical binding reference qualifies the concrete route, signer, or
resource-lineage head used for one access attempt. The latter may advance under the admitted
stable lineage without changing program semantics. It is not the private signer or writer
credential.

A `QualifiedPhysicalBinding<K>` is a sealed assembly object that combines:

- an immutable secret-free certificate reference registered under that stable lineage and
  implementation binding; and
- the private invoker handle and credential needed to use the concrete target.

The qualification registry retains immutable public certificates and their monotonic lineage
relation. The run-history store receives only a purpose-limited verifier for those certificates,
not signer, resource-mutation, or generic database authority. A public reference alone cannot
construct `QualifiedPhysicalBinding<K>` or call preparation.

### Typed lexical value

A value handle names an admission root or an exact producer that lexically dominates its consumer.
A branch-local value cannot escape its arm except through a certified same-type merge. A fan-out
lane value cannot escape before the join and retains its lane provenance afterward.

The certified program contains slots, not future content references:

```text
LexicalSlot<T> {
    lexical_path,
    complete_contract_ref,
    exact_producer_shape,
}

ExistingLexicalSlot<T> =
    one exact already defined LexicalSlot<T>;
    it cannot introduce a new producer shape

ExactStateFailureSlot<T, Contract, OccurrenceId> =
    LexicalSlot<T> whose producer shape is
        StateOutput(OccurrenceId, TypedFailure)
    where Contract == FailureContract::Typed(TypedContract)
    and whose complete_contract_ref == TypedContract.exact_ref

ExactSelectorSlot<T> =
    one ExistingLexicalSlot<T> whose complete contract is one
    registered closed canonical tagged sum

ExactVariantPayloadSlot<
    T,
    SelectorSlot,
    CanonicalTag,
    PayloadPath,
> =
    LexicalSlot<T> whose producer shape is
        VariantPayload(
            SelectorSlot,
            CanonicalTag,
            PayloadPath,
        )
    where SelectorSlot is ExactSelectorSlot<ClosedTaggedSum>
    and SelectorContract is the exact registered closed-sum contract
        object referenced by SelectorSlot.complete_contract_ref
    and VariantEntry is the unique entry in
        SelectorContract.exact_tag_payload_table at
        (CanonicalTag, PayloadPath)
    and T == VariantEntry.exact_payload_type
    and whose complete_contract_ref
        == VariantEntry.exact_payload_contract_ref

ExactDominatingCallerSlot<T, BoundaryId> =
    one ExistingLexicalSlot<T> in the caller region whose exact
    producer path lexically dominates BoundaryId

FragmentInputSlot<
    T,
    BoundaryId,
    ChildRootId,
    ChildRootContract,
    SourceSlot,
> =
    LexicalSlot<T> whose producer shape is
        FragmentInput(BoundaryId, ChildRootId, SourceSlot)
    where SourceSlot is
        ExactDominatingCallerSlot<T, BoundaryId>
    and whose complete contract is exactly ChildRootContract

ExactFragmentSuccessSlot<
    T,
    Contract,
    BoundaryId,
    SourceSlot,
> =
    LexicalSlot<T> whose producer shape is
        FragmentBoundary(BoundaryId, SuccessOutput, SourceSlot)
    and whose complete contract is exactly Contract

ExactFragmentFailureSlot<
    T,
    Contract,
    BoundaryId,
    SourceSlot,
> =
    LexicalSlot<T> whose producer shape is
        FragmentBoundary(BoundaryId, TypedFailure, SourceSlot)
    where Contract == FailureContract::Typed(TypedContract)
    and whose complete_contract_ref == TypedContract.exact_ref
```

A slot has no `ContentRef`, transition, or claim that execution already happened.
`ExactVariantPayloadSlot`, `FragmentInputSlot`, and the two fragment-boundary slots are certified
slot-to-slot derivation recipes; constructing one does not invoke the retained-reference
constructors below. The
callback-free fold resolves a slot to one `LexicalValueRef` only when the certified source is
active and its exact value has been committed. It then applies the certified recipe to construct
the corresponding `FragmentInputValueRef` or `FragmentBoundaryValueRef`. Admission,
expansion, and certification compare slots; Runtime/store settlement compare the resolved
references.
For `ExactVariantPayloadSlot`, neither the payload type nor its nominal retained contract is an
independent caller choice: both are looked up in the exact selector contract's certified
`(CanonicalTag, PayloadPath)` table. Byte-identical payloads registered under different nominal
contracts therefore cannot substitute for one another.

The retained reference algebra is:

```text
LexicalValueRef<T> =
    AdmissionRoot {
        root_id,
        value_ref: ContentRef<T>,
        contract_ref,
    }
  | StateOutput {
        occurrence_id,
        transition_result_role: SuccessOutput | TypedFailure,
        value_ref: ContentRef<T>,
        contract_ref,
    }
  | StructuralValue {
        structural_path,
        value_ref: ContentRef<T>,
        contract_ref,
        derivation:
            ArmValue {
                selected_arm_path,
                source: LexicalValueRef<T>,
                exact_contract:
                    contract_ref == source.contract_ref,
                exact_value_ref:
                    value_ref == source.value_ref,
            }
          | VariantPayload {
                selector: LexicalValueRef<ClosedSum>,
                canonical_tag,
                payload_path,
                exact_selector_entry:
                    selector_contract =
                        exact registered closed-sum contract object
                        referenced by selector.contract_ref,
                    variant_entry =
                        selector_contract.exact_tag_payload_table[
                            (canonical_tag, payload_path)
                        ],
                    T == variant_entry.exact_payload_type,
                    contract_ref
                        == variant_entry.exact_payload_contract_ref,
                exact_value_ref:
                    value_ref
                        == content_ref(
                               canonical_payload_bytes(
                                   selector.value_ref,
                                   canonical_tag,
                                   payload_path,
                               )
                           ),
            }
          | FragmentInput {
                boundary_id,
                child_input_root_id,
                source: LexicalValueRef<T>,
            }
          | FragmentBoundary {
                boundary_id,
                boundary_result_role:
                    SuccessOutput | TypedFailure,
                source: LexicalValueRef<T>,
            }
          | FanOutJoin<Output, Failure> {
                exact_result_type:
                    T == DeclaredOrderVector<
                        LaneOutcome<Output, Failure>,
                    >,
                lane_output_contract_ref,
                lane_failure_contract_ref,
                declaration_ordered_sources:
                    [LaneOutcomeRef<Output, Failure>]
                        where there is exactly one source for
                        every declared lane, no other source,
                        and every source has that group's exact
                        identity and output/failure contract refs,
                exact_value_ref:
                    value_ref
                        == content_ref(
                               canonical_declared_order_vector_bytes(
                                   declaration_ordered_sources,
                               )
                           ),
            },
    }

LaneOutcomeRef<Output, Failure> =
    ContentRef<
        CanonicalLaneOutcome {
            fan_out_group_path,
            stable_lane_key,
            declaration_ordinal,
            output_contract_ref,
            failure_contract_ref,
            outcome:
                LaneOutcome<
                    LexicalValueRef<Output>,
                    LexicalValueRef<Failure>,
                >,
        },
    >

ArmValueRef<Arm, T>::from(source) =
    LexicalValueRef<T>::StructuralValue {
        structural_path: Arm.selected_arm_path,
        value_ref: source.value_ref,
        contract_ref: source.contract_ref,
        derivation: ArmValue {
            selected_arm_path: Arm.selected_arm_path,
            source,
        },
    }

VariantPayloadValueRef<
    Selector,
    CanonicalTag,
    PayloadPath,
    T,
>::from(selector) =
    LexicalValueRef<T>::StructuralValue {
        require selector's exact retained canonical closed-sum object;
        require selector.tag == CanonicalTag;
        variant_entry =
            selector.contract.exact_tag_payload_table[
                (CanonicalTag, PayloadPath)
            ];
        require variant_entry.exact_payload_type == T;
        payload_bytes =
            exact canonical bytes at PayloadPath under CanonicalTag;
        validate payload_bytes with the exact registered contract object
            referenced by variant_entry.exact_payload_contract_ref;
        structural_path:
            Selector.structural_path + CanonicalTag + PayloadPath,
        value_ref: content_ref(payload_bytes),
        contract_ref: variant_entry.exact_payload_contract_ref,
        derivation: VariantPayload {
            selector,
            canonical_tag: CanonicalTag,
            payload_path: PayloadPath,
        },
    }

FanOutJoinValueRef<Group, Output, Failure>::from(
    declaration_ordered_sources
) =
    LexicalValueRef<
        DeclaredOrderVector<LaneOutcome<Output, Failure>>,
    >::StructuralValue {
        require the exact non-empty lane bijection, group identity,
        declaration order, and homogeneous contracts;
        join_bytes =
            canonical_declared_order_vector_bytes(
                declaration_ordered_sources,
            );
        structural_path: Group.structural_path,
        value_ref: content_ref(join_bytes),
        contract_ref: Group.join_contract_ref,
        derivation: FanOutJoin {
            declaration_ordered_sources,
            lane_output_contract_ref:
                Group.output_contract_ref,
            lane_failure_contract_ref:
                Group.failure_contract_ref,
        },
    }

FragmentBoundaryValueRef<Boundary, T>::from(source) =
    LexicalValueRef<T>::StructuralValue {
        exact_contract:
            source.contract_ref == Boundary.contract_ref,
        structural_path: Boundary.structural_path,
        value_ref: source.value_ref,
        contract_ref: Boundary.contract_ref,
        derivation: FragmentBoundary {
            boundary_id: Boundary.boundary_id,
            boundary_result_role:
                Boundary.result_role,
            source,
        },
    }

FragmentInputValueRef<Boundary, ChildRoot, T>::from(source) =
    LexicalValueRef<T>::StructuralValue {
        exact_contract:
            source.contract_ref == ChildRoot.contract_ref,
        structural_path:
            Boundary.body_region + ChildRoot.root_id,
        value_ref: source.value_ref,
        contract_ref: ChildRoot.contract_ref,
        derivation: FragmentInput {
            boundary_id: Boundary.boundary_id,
            child_input_root_id: ChildRoot.root_id,
            source,
        },
    }
```

All structural constructors are sealed fold operations. A variant-payload or join constructor
binds its derived canonical object in the same atomic object closure that first needs it; it cannot
name unbound bytes. `Match`, fragment boundaries, and fan-out joins may derive a lexical value and
its content reference, but they never become executable occurrences. The fold validates the
complete derivation, selected arm, selector tag, payload path, fragment-input substitution, source
contracts, canonical derived bytes, content reference, and lane order.
`ArmValue` retains a same-type value produced by the selected arm. `VariantPayload` extracts a
typed payload from the exact retained closed-sum selector and requires its certified tag, payload
path, and selector-table-derived payload contract. Payload bytes are validated by that exact
contract; their shape or byte identity never selects a nominal contract.

`ProducerBound<T>` generically means one exact current-run resolved certified success slot whose
allowed producer shape is `StateOutput(SuccessOutput)` or the exact
`FragmentBoundary(SuccessOutput)` declared by that consumer. Its constructors are sealed. It
excludes typed-failure producers, admission roots, arbitrary arm aliases, variant-payload
projections, fan-out joins, and embedded references copied from another run. When a domain needs
to consume such structural or embedded material, an explicit certified `Pure` state validates it
and emits a new current-run `StateOutput`—as `BindCurrentWalletReservation` does.

`FailurePlanBound<T, PlanIdentity>` is the separate sealed specialization available only inside
the exact selected failure plan whose certified `plan_identity` equals `PlanIdentity`. The fold
constructs it only by resolving that plan's exact current slot: initially the selected
`StateOutput(TypedFailure)` or exact `FragmentBoundary(TypedFailure)`, and after an explicit mapper
the `StateOutput(SuccessOutput)` from that exact `Pure + Never` mapper occurrence. Every mapper and
the designated handler receives this sealed runtime view for the same plan identity; the
certified program itself retains only the corresponding slots. No ordinary state input can
request a typed-failure producer. Structural compatibility never substitutes for the exact
producer shape, plan, and identity.

A lane tail constructs exactly one canonical `LaneOutcomeRef` around its selected success or
failure `LexicalValueRef`; its content-addressed bytes bind the fan-out group path, stable lane
key, declaration ordinal, and exact homogeneous output/failure contracts. This is structural
normalization, not a transition. The fan-out join requires a bijection with the declared lanes,
consumes those wrappers in declaration order, and rejects an omitted, duplicate, foreign,
misordered, or contract-substituted lane. Neither the lane result nor the join receives an
executable occurrence identity.

### State outcome

A state callback can only propose semantic material:

```text
ProposedStateOutcome<Output, Failure> =
    Success(Output)
  | Failure(Failure)

StateOutcome<OutputRef, FailureRef> =
    Success(OutputRef)
  | Failure(FailureRef)
```

A proposal has no producer identity, lexical authority, or retained form. After validating and
content-addressing the proposed value under the exact admitted contract, the store's accepted
state-transition append alone constructs the nominal committed
`StateOutcome<LexicalValueRef<Output>, LexicalValueRef<Failure>>`. A rejected candidate or
callback result that never commits constructs no `StateOutcome`. For `FailureContract::Never`,
both the proposed failure variant and the committed failure reference are structurally illegal.

A state failure is typed domain truth. It selects a failure continuation; it is not automatically
the terminal run result. Only the root `OperationOutcome` determines whether the run closes with
success or failure.

`StateOutcome` and `OperationOutcome` are distinct nominal authority types despite their identical
two-tag encoding. A committed transition produces the former; the callback-free fold derives the
latter only at the operation root for `RunClosed`. Neither can be substituted for the other by
schema shape.

`StateOutcome` is a private state-bound lexical result and discriminator for the exact
`StateBinding`. The fold derives its sealed continuation from the variant;
the value is not a control instruction. Its failure variant can be consumed only by that
boundary's sealed failure plan and never escapes through a fan-out join or ordinary author-visible
`Match`. `LaneOutcome` is a distinct nominal lexical type produced only after lane-scoped
handling; its identical two-tag shape does not make the two authority types interchangeable.

These nominal kernel envelopes retain references rather than embedding an arbitrary
`Failure` value. When the boundary failure contract is `Never`, the failure tag and reference slot
are structurally illegal and no decoder for a failure payload exists. The success tag remains
canonical. Thus an infallible operation or lane does not need a fictional retained schema for an
uninhabited generic parameter.

### Fallible state

Fallibility is determined from the explicit certified failure contract:

```text
StateFallibility<S> =
    Infallible
        when S.failure_contract == FailureContract::Never
  | Fallible<S.Failure>
        when S.failure_contract == FailureContract::Typed(
            S.failure_retained_value_contract
        )
```

This is a static property of the admitted state contract. It is not inferred from `Pure`, `Read`,
or `Effect`; from whether the Rust implementation happens to use an uninhabited type; or from
whether a particular occurrence fails at runtime. Any execution kind may be infallible or
fallible.

`FailureContract::Never` resolves to one frozen, domain-separated
`KERNEL_NEVER_FAILURE_CONTRACT_REF`, but it is deliberately not a `RetainedValueContract`: the
current schema system has no honest uninhabited value schema. It has no decoder, `MfmValue`
implementation, retained slot, or producer. Rust `!`, `Infallible`, `()`, an empty struct, a
structurally similar schema, a separately registered look-alike alias, or an implementation that
“never currently fails” does not prove infallibility. The kernel zero-variant `Never` source type
is accepted only through the sealed `FailureContract::Never` registration path.

Every `FailureContract::Typed` contract is fallible. Its `RetainedValueContract` remains the one
producer-independent authority over schema, semantic type, retained role, media type, and evidence
contract; its canonical content reference is the exact contract identity. A fallible occurrence
must carry one sealed
exhaustive `FailurePlan`; an infallible occurrence must carry `NoFailure` and cannot be given a
handler. A dynamically committed `StateOutcome::Failure` is the event that selects the fallible
occurrence's plan. Its producer-bound value must match the state's complete admitted failure
contract by exact identity, while `ValueRef` separately binds the exact state-failure producer
role and occurrence. Structural compatibility or coercion is insufficient. Operational access outcomes,
integrity faults, persistence failures, and possible-entry ambiguity remain outside this
domain-failure classification.

A failure mapper is an ordinary `Pure` state with an exact output failure-contract reference.
Changing a source or mapped failure contract, mapper implementation, or retained-value contract
changes the relevant manifests, expanded-program identity, and certification result.

The rule is orthogonal to execution kind:

| State kind | `FailureContract::Never` | `FailureContract::Typed` |
| --- | --- | --- |
| `Pure` | `Infallible`; `NoFailure`; no handler | `Fallible<E>`; exactly one sealed failure plan |
| `Read` | `Infallible`; access still uses the audited bracket; no handler | `Fallible<E>`; exactly one sealed failure plan |
| `Effect` | `Infallible`; access and possible-entry rules still apply; no handler | `Fallible<E>`; exactly one sealed failure plan |

For a `Read` or `Effect` whose failure contract is `Never`, a committed `SafeFailure` cannot synthesize a typed failure or
enter a handler. Capability/state qualification must prove an exhaustive deterministic disposition
for every admitted `SafeFailure` variant. An infallible state may advertise such a variant only
when every valid instance maps to its certified success output; a pairing that needs an ordinary
negative state result is rejected. `InvalidEvidence` remains the response to malformed,
inconsistent, or tampered evidence, not the declared meaning of an expected `SafeFailure`.
Unresolved access and integrity dispositions remain blocking under their own protocols.
“Infallible” therefore means only “cannot produce domain `StateOutcome::Failure`,” not immunity
from callback, codec, store, integrity, or crash faults.

### Failure handler

An ordinary injected `Pure` state that consumes the exact producer-bound state or fragment-boundary
failure and returns one closed scope-defined failure route. It is infallible and performs no IO.

An operation, fragment, or fan-out lane with a typed failure contract may register an exact finite
source-contract-to-mapper table. `.or_default()` selects exactly one matching `Pure + Never`
mapper; it is rejected for zero or multiple matches. A `Never` scope has no table and requires
explicit total recovery for every fallible call. A custom handler may select an explicit recovery
branch. The recovery states themselves are ordinary states.

### Semantic execution kind

The certified property of a state:

- `Pure`: deterministic local computation with no semantic external IO.
- `Read`: one typed external observation that does not intentionally mutate the target.
- `Effect`: one typed operation that may mutate an external target or consume an exclusive
  capability.

This is semantic metadata, not three independent Runtime engines.

### Fan-out

An explicit bounded set of declaration-ordered lanes. Cardinality and lane identity are frozen
before admission. Each lane contains only `Pure` and `Read` states, has no cross-lane references,
and produces exactly one typed result. The containing block continues only after collect-all
join. Fan-out may nest to certified depth two; the transitive `Pure | Read` restriction and global
expanded-lane bound apply at both levels.

Concurrency is permitted operationally; semantic result order is always declaration order.

### Expansion policy

A pure registered transformation that replaces one eligible state slot with a typed structured
fragment. A wrapper receives an affine protected slot that cannot be cloned. It may guard the slot
through an explicit branch, but it cannot duplicate it or move it relative to sibling
declarations.

### Access kind

The sealed Runtime protocol parameter:

```text
AccessKind = Read | Effect
```

Prior-run fact selection is an ordinary `Read` bound to the sealed, purpose-limited RunHistory
fact scanner. Its certified request carries the completeness and source-manifest contract. It uses
the same access bracket and does not create a fourth state or access kind.

### Safe failure

A reviewed, bounded, redaction-safe definite operational completion for which no typed response
is available or appropriate. It contains enough qualified evidence for the state to settle
deterministically. It does not inherently prove non-entry and never authorizes another attempt.

### Protected non-application and possible entry

- `SupersededBeforeEntry`: qualified evidence that the protected semantic mutation or exclusive
  capability consumption was not applied. The resource authority may have executed a read or
  fencing transaction to prove that fact.
- `EntryUnknown`: the available evidence cannot exclude target entry.

On recovery, an unmatched effect authorization is reported as possible-entry ambiguity equivalent
to `EntryUnknown`. An unmatched read is reported as `ReadCompletionUnknown`: no mutation ambiguity
exists, but no response was committed either. The persisted fold state remains
`Authorized<Effect>` or `Authorized<Read>` because history cannot prove process loss. Absence of an
observation never proves non-application.

### One bounded semantic interaction

One already-selected operation with one immutable typed request and one closed completion. An
adapter may use the minimum lower-level primitives needed for that operation, but it may not
select another semantic operation, retry, poll, fail over, fold history, or terminalize a
workflow.

### Resource authority

A narrow durable adapter/store that serializes a cross-run invariant that no individual run
journal can prove. The wallet-nonce authority is the current example.

### Adapter

Private live-crate glue that consumes one Runtime authorization, performs one selected semantic
operation using lower primitives, and totalizes every normal completion. It owns no program,
cursor, scheduling, failure-handler, or run-history lifecycle.

### Transport

A reusable mechanism for bounded encoding, exchange, and checked decoding. It owns no MFM journal,
state, scheduling, semantic retry, or domain progression.

### Semantic and operational telemetry

- Semantic audit data affects or attests run meaning and is represented by ordinary certified
  states, facts, access records, or explicit effects.
- Operational logs, metrics, and spans observe redacted Runtime or committed-history events and
  never affect the program cursor or outcome.

## Required Guarantees

### G-01: One structured execution authority

Only the admitted `CertifiedProgram` defines legal state order, branches, fan-out lanes, root
outcomes, capabilities, and bindings. No graph, adapter, application callback, or runtime-origin
flag is a second authority.

### G-02: Declaration order is semantic

Outside `FanOut`, at most one executable state occurrence is current. A later declaration cannot
execute, authorize, or settle before the current declaration resolves and structured control
advances.

### G-03: Expansion is pure, finite, and visible

Identical canonical authoring input, registry, expansion profile, and manifests produce
byte-identical expanded bytes and hashes. Expansion performs no ambient IO. Every injected state
is present in the admitted program with exact structural identity and bindings.

### G-04: Every fallible boundary is handled

Every state or fragment boundary whose certified failure contract is `Typed` has one sealed
`FailurePlan` after expansion. That plan either
enters its one explicit or lexical-default handler, or propagates through the exact affine
fragment boundary to one eventual call-site handler. Its protected failure cannot do both,
directly bypass the handler through normal control, or escape to an unrelated scope. A distinct
failure newly committed by an intervening pre/post/support state follows that state's own exact
plan and may causally supersede the protected failure. A source with `FailureContract::Never` has
`NoFailure` and no handler.

An operation, lane, or fragment scope whose own failure contract is `Never` cannot supply a mapper
into that uninhabited failure contract.
Every fallible boundary directly owned by such a scope must have an explicit total handler whose
every route recovers. A nested fragment with a typed failure contract may
handle its own internal failures, but its call-site failure in the `Never` scope must
likewise recover explicitly. Certification rejects `.or_default()` and any failure-producing
scope result where the lexical default is unavailable.

### G-05: One run-history mutation owner

Only Runtime instances hold production `RunHistoryWriter` authority. Applications, schedulers,
states, adapters, transports, replay, CLI, REST, and telemetry observers receive no append
authority.

### G-06: Authorization precedes possible entry

No read, effect, resource transaction, signer operation, provider call, or purpose-limited fact
scan begins until its exact authorization positively commits.

An existing authorization, ambiguous append acknowledgement, or stale candidate cannot mint live
authority.

### G-07: One affine invocation and one closed normal completion

A newly committed authorization mints one non-cloneable `Authorized<K>`. Consuming it permits
exactly one entry into the registered invoker for the exact operation and binding. Typing cannot
prove how many lower-level calls arbitrary adapter code makes; adapter qualification, bounded
operation contracts, and fault-injection tests must prove the declared no-retry/no-duplication
behavior.

After the registered invoker accepts that authority, it has no outer normal error channel. Every
surviving return becomes one bounded persistable completion.

### G-08: Pending observation cannot escape

The invoker completion is immediately owned by Runtime as `PendingObservation<K>`. No normal
successful drive result, state callback, application result, or live value can escape before exact
observation commit or identical-content resolution.

### G-09: Observation is exactly linked

Every observation names exactly one authorization and preserves its access kind, occurrence,
semantic call, operation, binding, request, cursor anchor, and outcome contract. At most one
observation exists per authorization.

### G-10: Only the current cursor may advance

The store accepts a transition or authorization only for the current sequential state occurrence
or one eligible state occurrence inside an active fan-out lane. Runtime cannot ask the store to
choose another state occurrence, and no lane or group can be named as a transition target.

### G-11: Unresolved same-occurrence access overlap is unrepresentable

An Effect occurrence with an unmatched authorization or committed `EntryUnknown` evidence has no
transition that creates another authorization or advances to a later step. This prevents a second
possible external mutation.

A new authorization for the same semantic occurrence is legal only after the previous
authorization committed qualified `SupersededBeforeEntry` evidence and the fold produced the
next `Refreshable` attempt ordinal. It must be prepared by a worker holding the currently
qualified physical binding.

An unmatched Read authorization likewise remains the exact current access as
`ReadCompletionUnknown`. The initial contract authorizes no same-occurrence re-read, timeout
supersession, or synthetic completion: a later read could select a different time-varying value.
A separately admitted run may execute independently. This is a selected conservative recovery
policy, not an unresolved scheduling choice.

### G-12: Branch and fan-out control is deterministic

The store derives a `Match` arm from the canonical tag of a committed closed sum. The exact
`CertifiedProgram` closure contains the exhaustive tag-to-arm table; the caller does not write a
branch choice and the store runs no discriminator callback.

Fan-out lanes and joined results retain declaration order regardless of authorization, completion,
or append order. Effects are rejected transitively inside fan-out. Nesting beyond certified depth
two is rejected.

### G-13: Non-domain evidence cannot become domain truth

Only a committed typed state or fragment-boundary failure may enter a failure handler. Raw
provider text, audit-only platform faults, integrity violations, process loss, store failure,
unmatched authorization, and `EntryUnknown` are not failure inputs.

### G-14: Ordinary definite failures terminate or follow explicit recovery

Every expected definite state-facing operational completion is admitted as either a reviewed typed
`Returned(Response)` or a reviewed capability `SafeFailure`. The state maps it deterministically
to success or typed failure; `InvalidEvidence` is reserved for an invalid instance, not an expected
valid disposition. Capability/state qualification rejects any expected negative `SafeFailure`
that the state's exact failure contract cannot represent. A typed failure enters the
explicit/default operation failure path and cannot leave the run permanently open merely because
it is an ordinary error.

A closed failed run grants no authority over a later run, and its run-history status cannot block
admission or execution of a later `run_id`. Independent resource authorities may still enforce
their durable invariants.

### G-15: Store validation remains authoritative

Runtime proposes sealed append material. The store alone validates legal record shape, exact
predecessor, logical-key uniqueness, object closure, cursor legality, provenance, access linkage,
semantic versus journal heads, and closure before atomic append.

### G-16: Cross-run resources have one narrow authority

Every resource domain has one qualified durable owner of its uniqueness rule. Every actor capable
of using the protected EVM sender either uses that same authority or is permanently fenced out.

### G-17: Normal resource rotation cannot poison semantics

A protected resource `Effect` request first enters its exact physical target through a sealed
current target-bound capability, then resolves its permanent semantic operation key before any new
mutation. An already committed byte-identical result on that admitted target is the original
operation result; internal `ExistingSame` resolution does not perform another mutation or create
another Runtime authorization. A revoked or stale mutation capability, or a stale binding with no
exact result, has only the atomic `SupersededBeforeEntry(public_lineage_head_ref)` outcome. That
proof cannot construct state output, state failure, or semantic cursor advancement. The
callback-free fold alone converts it into a sealed `Refreshable` state with the next attempt
ordinal. Only a newly assembled worker holding the current target-bound capability may prepare and
authorize that attempt; the stale worker cannot self-upgrade. Resource `Read` staleness follows its
separate definite `SafeFailure` contract and never constructs `Refreshable`.

### G-18: No secrets

Authored/expanded programs, manifests, requests, observations, reservation evidence, state
failures, facts, histories, traces, exports, telemetry projections, and public results contain no
credentials, private keys, signatures, raw signed envelopes, provider-controlled text,
secret-bearing endpoints, or secret paths.

### G-19: No generic executor survives

No executor-owned history, fold, scheduler, generic resource fence, delivery plan, target
authority, frontier, tombstone, or compatibility path remains after the cutover. The narrow
deployment physical fence specified for the wallet-nonce authority is not an executor.

## Target Architecture

### Responsibility placement

| Concern | Sole owner |
| --- | --- |
| Declaration order, branch/fan-out shape, child composition, operation failure contract | operation authoring |
| Trusted program/predicate contracts, required expansion profile, versions, and security/control coverage | qualified entry-point admission policy |
| Pure state and policy injection | deterministic expansion |
| Static structure, type, bounds, capability closure, profile coverage, and provenance | certification |
| Domain request authorship and observation interpretation | state |
| One run's current action | Runtime over a store-minted verified cursor |
| Run-history mutation orchestration | Runtime |
| Legal append, exact-head CAS, callback-free fold, cursor, and closure | `RunHistory` store |
| One already-selected live operation | adapter |
| Protocol exchange | transport |
| Signer generation and secret custody | qualified signer |
| Cross-run sender/nonce uniqueness and physical generation | wallet-nonce authority |
| Permanent wallet-domain activation binding and current store-incarnation head | qualified wallet-activation registry administrative plane |
| Exclusive physical target, writer-epoch, and sender-path fencing across promotion | deployment-owned authoritative fence control plane |
| EVM progression and terminal meaning | EVM states and expansion |
| Recorded verification | store/replay over committed evidence |
| Prior-run fact scan completeness | sealed purpose-limited RunHistory scanner |
| Best-effort logs, metrics, and spans | non-authoritative observers |
| Authentication and rendering | app, CLI, and REST |

No row is owned by an executor.

### Expanded and certified structured program algebra

The certified representation is one recursively structured tree:

```text
OperationProgram<Output, Failure> =
    OrderedBlock<
        SequentialPolicy<fan_out_depth = 2>,
        Output,
        Failure,
    >

OrderedBlock<Policy, Success, ScopeFailure> {
    ordered_bindings:
        [Binding<Policy, ScopeFailure>],
    tail:
        BlockTail<Success, ScopeFailure>,
}

BlockTail<Success, ScopeFailure> =
    Normal(TypedExpression<Success>)
  | ScopeFailure(TypedExpression<ScopeFailure>)

Binding<Policy, ScopeFailure> =
    StateBinding
  | MatchBinding
  | FanOutBinding where Policy.remaining_fan_out_depth > 0
  | FragmentBinding
```

Only `StateBinding` is executable. `MatchBinding`, `FanOutBinding`, and `FragmentBinding` are
structural. Every nested block has exactly one normal result type and inherits one exact lexical
failure channel. There is no independent general result axis, non-local successful result, jump,
or executable outcome form.

`BlockTail` is a structural expression selected only after the ordered bindings on its active
path complete. It has no occurrence identity or history record. Specialized pre-handler and
propagation blocks below deliberately expose narrower tails so they cannot take the general
lexical-failure exit.

The sealed policies are:

```text
SequentialPolicy<remaining_fan_out_depth> {
    allowed_state_kinds: Pure | Read | Effect,
}

FanOutPolicy<remaining_fan_out_depth> {
    allowed_state_kinds: Pure | Read,
}
```

Entering a fan-out lane changes `SequentialPolicy<D>` or `FanOutPolicy<D>` to
`FanOutPolicy<D - 1>`. No constructor can increase the remaining depth or restore `Effect`.
Certification additionally enforces the entry point's total expanded-occurrence, total-lane,
branch-depth, and per-group bounds. The initial production limit is fan-out depth two.

A state binding is:

```text
StateBinding<Policy, ScopeFailure> {
    output_local,
    stable_label,
    call where call.kind is in Policy.allowed_state_kinds,
    failure_boundary:
        NoFailure {
            exact_contract: call.failure_contract
                == FailureContract::Never,
            no_failure_slot,
            no_failure_plan,
        }
      | TypedFailure {
            exact_contract: call.failure_contract
                == FailureContract::Typed,
            source_slot:
                ExactStateFailureSlot<
                    call.Failure,
                    call.failure_contract,
                    call.occurrence_id,
                >,
            plan:
                FailurePlan<
                    Policy,
                    source_slot,
                    call.Output,
                    ScopeFailure,
                >,
        },
}
```

Its success binds `output_local` and continues with the next declaration. Its typed failure enters
the sealed plan. `StateOutcome` is private to this structural split and is never a public
`Match` selector.

A match binding is:

```text
MatchBinding<Policy, Value, ScopeFailure> {
    output_local: TypedLocal<Value>,
    stable_label,
    selector: ExactSelectorSlot<ClosedTaggedSum>,
    exhaustive_arms: [
        TaggedArm {
            canonical_tag,
            stable_arm_label,
            completion:
                Continue {
                    body: OrderedBlock<Policy, Value, ScopeFailure>,
                }
              | ScopeFailure {
                    body:
                        ConstrainedFailureBlock<
                            Policy,
                            ScopeFailure,
                        >,
                },
        },
    ],
}
```

`Continue` and `ScopeFailure` are structural arm shapes, not instructions. A continuing arm
produces the binding's one exact value contract. A failure arm produces only the current lexical
scope's exact failure contract. No arm may complete an enclosing scope successfully. If a branch
determines the operation's final successful value, that `Match` is placed at the lexical tail or
the remaining sequence is nested under each continuing arm.

A fan-out binding is:

```text
FanOutBinding<
    Policy,
    LaneOutput,
    LaneFailure,
    ScopeFailure,
> {
    exact_policy: Policy.remaining_fan_out_depth > 0,
    output_local:
        TypedLocal<
            DeclaredOrderVector<
                LaneOutcome<LaneOutput, LaneFailure>,
            >,
        >,
    stable_group_label,
    nonzero_bound,
    ordered_lanes: [
        Lane {
            stable_lane_key,
            derived_declaration_ordinal,
            body:
                OrderedBlock<
                    FanOutPolicy<
                        Policy.remaining_fan_out_depth - 1,
                    >,
                    LaneOutput,
                    LaneFailure,
                >,
        },
    ],
    exact_homogeneous_lane_contracts,
}
```

Each lane normal result is nominally wrapped as `LaneOutcome::Success`; its lexical failure is
nominally wrapped as `LaneOutcome::Failure`. The join produces exactly one wrapper per declared
lane in declaration order. The wrappers and join are structural values, not occurrences.

An expanded child call, semantic capability, or wrapper occupies one fragment binding:

```text
FragmentBinding<Policy, Input, Output, Failure, ScopeFailure> {
    fresh_lexical_region,
    output_local,
    stable_label,
    boundary:
        FragmentBoundary<Input, Output, Failure>,
    input_bindings:
        declaration_ordered [
            ChildInputRoot<T>
                -> FragmentInputSlot<
                    T,
                    boundary.boundary_id,
                    ChildInputRoot<T>.root_id,
                    ChildInputRoot<T>.contract_ref,
                    ExactDominatingCallerSlot<
                        T,
                        boundary.boundary_id,
                    >,
                >
        ],
    body:
        OrderedBlock<Policy, Output, Failure>,
    success_slot:
        ExactFragmentSuccessSlot<
            Output,
            boundary.success_contract,
            boundary.boundary_id,
            ExactNormalTailSlotOf<body>,
        >,
    failure_boundary:
        NoFailure {
            exact_contract: boundary.failure_contract
                == FailureContract::Never,
            no_failure_slot,
            no_failure_plan,
        }
      | TypedFailure {
            exact_contract: boundary.failure_contract
                == FailureContract::Typed,
            source_slot:
                ExactFragmentFailureSlot<
                    Failure,
                    boundary.failure_contract,
                    boundary.boundary_id,
                    ExactFailureTailSlotOf<body>,
                >,
            plan:
                FailurePlan<
                    Policy,
                    source_slot,
                    Output,
                    ScopeFailure,
                >,
        },
}
```

Input binding covers every declared child/fragment input root exactly once, admits no extra root,
requires exact contract equality, and preserves the caller source slot under the fresh body
region. `ExactNormalTailSlotOf<body>` and, for a typed boundary,
`ExactFailureTailSlotOf<body>` select the exact certified tail-expression slots; neither claims
that a path is active or a value is committed. The success and failure slots bind those sources
symmetrically to the exact boundary identity and role. The fragment body owns the exact failure
mappings for states it injects. At fold time, normal completion resolves the certified success
slot and binds the boundary output local. Its lexical failure resolves the certified failure slot
and enters the call site's one plan. Entering or leaving a fragment creates no transition, copied
value, or child `RunClosed`.

The certified failure contract is:

```text
FailureContract<Failure> =
    Never {
        exact_type: Failure == kernel::Never,
        exact_ref: KERNEL_NEVER_FAILURE_CONTRACT_REF,
        no_retained_value_contract,
        no_codec,
        no_producer,
    }
  | Typed {
        exact_type: Failure implements MfmValue,
        contract: RetainedValueContract<Failure>,
        exact_ref:
            content_ref(annex_canonical_bytes(contract)),
    }
```

`RetainedValueContract` remains the sole producer-independent descriptor of an inhabited typed
value. Producer shape and identity remain in certified `LexicalSlot` and resolved
`LexicalValueRef`; no second failure descriptor duplicates them. The kernel source type for
`Never` is a zero-variant enum without an `MfmValue` implementation. A normal unit, empty struct,
empty-looking schema, or distinct contract alias is inhabited or has the wrong reference and is
therefore not `Never`.

Failure plans are:

```text
FailurePlanIdentity<
    SourceBindingIdentity,
    SourceFailureSlot,
    PlanStructuralPath,
> =
    domain_separated_hash(
        "mfm.failure-plan.v1",
        SourceBindingIdentity,
        SourceFailureSlot,
        PlanStructuralPath,
    )

FailurePlan<Policy, SourceFailureSlot, RecoveredOutput, ScopeFailure> =
    Handled {
        lexical_region,
        plan_structural_path,
        plan_identity:
            FailurePlanIdentity<
                exact_binding_of(SourceFailureSlot),
                SourceFailureSlot,
                plan_structural_path,
            >,
        before_handler:
            ConstrainedPreHandlerBlock<
                Policy,
                plan_identity,
                lexical_region,
                SourceFailureSlot,
                ScopeFailure,
            >,
        handler:
            StateBinding<Policy, ScopeFailure> {
                stable_label,
                output_local,
                call {
                    kind: Pure,
                    certified_input_slot:
                        ExactResultSlotOf<before_handler>,
                    output: ClosedHandlerRoute,
                    failure_contract: Never,
                },
                failure_boundary: NoFailure,
            },
        route:
            DefaultPropagation<ScopeFailureContract> {
                exact_enclosing_scope_failure_contract:
                    FailureContract::Typed(
                        ScopeFailureContract,
                    ),
                exact_handler_output_type:
                    handler.Output
                        == DefaultScopeFailureRoute<
                            ScopeFailure,
                            ScopeFailureContract,
                        >,
                continuation:
                    DefaultScopeFailureContinuation<
                        Policy,
                        ExactOutputSlotOf<handler>,
                        ScopeFailure,
                        ScopeFailureContract,
                    >,
            }
          | CustomRecovery {
                continuation:
                    ExhaustiveHandlerRoute<
                        Policy,
                        ExactOutputSlotOf<handler>,
                        RecoveredOutput,
                        ScopeFailure,
                    >,
            },
    }
  | Propagate<
        LexicalRegion,
        BoundaryId,
        BoundaryFailure,
        BoundaryFailureContract,
    > {
        plan_structural_path,
        plan_identity:
            FailurePlanIdentity<
                exact_binding_of(SourceFailureSlot),
                SourceFailureSlot,
                plan_structural_path,
            >,
        affine_enclosing_boundary:
            EnclosingFragmentBoundaryToken<
                LexicalRegion,
                BoundaryId,
                BoundaryFailureContract,
            >,
        mapping_chain:
            ExactAffinePureMappingChain<
                plan_identity,
                LexicalRegion,
                SourceFailureSlot,
                ExistingLexicalSlot<BoundaryFailure>,
            >,
        boundary_slot:
            ExactFragmentFailureSlot<
                BoundaryFailure,
                BoundaryFailureContract,
                BoundaryId,
                mapping_chain.target_slot,
            >,
        exact_target_contract:
            contract_ref(mapping_chain.target_slot)
                == BoundaryFailureContract.retained_contract_ref,
        where BoundaryFailureContract == FailureContract::Typed,
    }
```

The identity is derived, never author supplied. The exact source binding and its plan structural
path therefore determine one plan even when another plan uses the same failure value contract.
The `CertifiedProgram` contains only the displayed input slot. After the current committed failure
selects this exact plan, the fold alone may resolve that slot as
`FailurePlanBound<value_type(slot), plan_identity>` for the handler callback.

The compact certified slot types used above are:

```text
ConstrainedPreHandlerBlock<
    Policy,
    PlanIdentity,
    LexicalRegion,
    SourceFailureSlot,
    ScopeFailure,
> {
    exact_plan_identity: PlanIdentity,
    exact_lexical_region: LexicalRegion,
    exact_source: SourceFailureSlot,
    direct_scope_failure_tail: Forbidden,
    ordered_bindings:
        [PreHandlerBinding<Policy, ScopeFailure> whose normal structural paths
         contain no ScopeFailure tail or arm],
    protected_failure_chain:
        ExactAffinePureMappingChain<
            PlanIdentity,
            LexicalRegion,
            SourceFailureSlot,
            ExistingLexicalSlot,
        >,
    only_normal_tail:
        ExistingLexicalSlot {
            result_slot:
                protected_failure_chain.target_slot,
            exact_source:
                result_slot == SourceFailureSlot
                    when protected_failure_chain is ZeroLink,
            exact_mapper_output:
                result_slot
                    == protected_failure_chain.last.output_slot
                    when the chain is NonEmpty,
            exact_handler_input_contract:
                contract_ref(result_slot)
                    == enclosing_handled_plan
                       .handler.call.input_contract_ref,
        },
    newly_committed_failure:
        may leave only through that producing binding's exact
        FailurePlan and is a distinct recorded causal failure,
}

PreHandlerBinding<Policy, ScopeFailure> =
    the sealed subset of Binding<Policy, ScopeFailure>
    whose recursively nested normal-control structure has no
    direct ScopeFailure completion; any newly committed state
    failure follows only that state's own exact FailurePlan

ExactResultSlotOf<PreHandlerBlock> =
    PreHandlerBlock.only_normal_tail.result_slot
        retaining the exact certified derivation from
        PreHandlerBlock.exact_source through
        PreHandlerBlock.protected_failure_chain

ExactOutputSlotOf<HandlerStateBinding> =
    LexicalSlot<
        HandlerStateBinding.Output,
    > whose producer shape is
        StateOutput(
            HandlerStateBinding.occurrence_id,
            SuccessOutput,
        )
      and whose complete contract equals
        HandlerStateBinding.output_contract_ref

ClosedHandlerRoute =
    one registered closed tagged sum whose tags are exhausted
    by the exact continuation table

ExhaustiveHandlerRoute<
    Policy,
    HandlerOutputSlot,
    RecoveredOutput,
    ScopeFailure,
> {
    selector: HandlerOutputSlot,
    selector_contract:
        exact registered closed-sum contract object referenced by
        HandlerOutputSlot.complete_contract_ref,
    enclosing_scope_failure_contract:
        exact enclosing owned scope FailureContract<ScopeFailure>,
    exact_tag_table: [
        tag -> Recover {
            body:
                OrderedBlock<
                    Policy,
                    RecoveredOutput,
                    ScopeFailure,
                >,
        }
      | tag -> ScopeFailure {
            payload_path,
            exact_variant_payload_slot:
                ExactVariantPayloadSlot<
                    ScopeFailure,
                    HandlerOutputSlot,
                    tag,
                    payload_path,
                >,
            where enclosing_scope_failure_contract
                == FailureContract::Typed(ScopeFailureContract)
            and selector_contract.exact_tag_payload_table[
                    (tag, payload_path)
                ].exact_payload_type
                == ScopeFailure
            and selector_contract.exact_tag_payload_table[
                    (tag, payload_path)
                ].exact_payload_contract_ref
                == ScopeFailureContract.exact_ref,
            body:
                ConstrainedFailureBlock<
                    Policy,
                    ScopeFailure,
                >,
        },
    ],
    when ScopeFailure == kernel::Never:
        every tag is Recover,
}

ConstrainedFailureBlock<Policy, ScopeFailure> {
    ordered_bindings:
        [Binding<Policy, ScopeFailure>],
    tail_slot:
        one exact ExistingLexicalSlot<ScopeFailure>
        defined and active in this arm,
    only_tail:
        ScopeFailure(
            ExactTypedExpression<
                ScopeFailure,
                tail_slot,
            >
        ),
}

DefaultScopeFailureRoute<
    ScopeFailure,
    ScopeFailureContract,
> =
    one registered closed tagged sum with exactly:
        Propagate(ScopeFailure)
    whose canonical Propagate payload entry has
        exact_payload_type == ScopeFailure
    and exact_payload_contract_ref == ScopeFailureContract.exact_ref

DefaultScopeFailureContinuation<
    Policy,
    HandlerOutputSlot,
    ScopeFailure,
    ScopeFailureContract,
> {
    selector: HandlerOutputSlot,
    enclosing_scope_failure_contract:
        exact enclosing owned scope
        FailureContract::Typed(ScopeFailureContract),
    selector_contract:
        exact registered
        DefaultScopeFailureRoute<
            ScopeFailure,
            ScopeFailureContract,
        > contract object referenced by
        HandlerOutputSlot.complete_contract_ref,
    exact_tag_table having exactly:
        Propagate -> ScopeFailure {
            exact_payload_contract:
                selector_contract.exact_tag_payload_table[
                    (Propagate, canonical_propagate_payload_path)
                ] == {
                    exact_payload_type: ScopeFailure,
                    exact_payload_contract_ref:
                        ScopeFailureContract.exact_ref,
                },
            payload_slot:
                ExactVariantPayloadSlot<
                    ScopeFailure,
                    HandlerOutputSlot,
                    Propagate,
                    canonical_propagate_payload_path,
                >,
            exact_scope_contract:
                payload_slot.complete_contract_ref
                    == ScopeFailureContract.exact_ref,
            body.ordered_bindings: [],
            body.tail_slot: payload_slot,
            body.only_tail:
                ScopeFailure(
                    ExactTypedExpression<
                        ScopeFailure,
                        payload_slot,
                    >
                ),
        }
}

EnclosingFragmentBoundaryToken<
    LexicalRegion,
    BoundaryId,
    BoundaryFailureContract,
> =
    non-cloneable token minted only for that exact fragment body

ExactAffinePureMappingChain<
    PlanIdentity,
    LexicalRegion,
    SourceFailureSlot,
    TargetFailureSlot,
> =
    ZeroLink {
        exact_plan_identity: PlanIdentity,
        exact_lexical_region: LexicalRegion,
        exact_contract_equality:
            contract_ref(SourceFailureSlot)
                == contract_ref(TargetFailureSlot),
        exact_target:
            TargetFailureSlot == SourceFailureSlot,
    }
  | NonEmpty {
        exact_plan_identity: PlanIdentity,
        exact_lexical_region: LexicalRegion,
        source: SourceFailureSlot,
        links: [
            CertifiedMapperBinding {
                binding:
                    StateBinding {
                        kind: Pure,
                        failure_boundary: NoFailure,
                    },
                exact_plan_identity: PlanIdentity,
                active_lexical_region: LexicalRegion,
                input_slot,
                input_contract_ref,
                output_slot,
                output_contract_ref,
            },
        ],
        exact_adjacency:
            first.input_slot == source
            and each next.input_slot == previous.output_slot
            and every adjacent contract is exactly equal,
        exact_final_output:
            last.output_slot == TargetFailureSlot,
    }
```

For either chain form, `target_slot` is exactly `TargetFailureSlot`; it never denotes a new
producer kind. Every link belongs to the chain's one lexical region and one selected plan. The
fold constructs a `FailurePlanBound<value_type(input_slot), PlanIdentity>` view only when that
exact mapper is current, from that exact resolved input slot, and under that exact
`PlanIdentity`; the view is not a certified-program field.

`NoFailure` is constructible only from `FailureContract::Never`, contains no source slot or plan,
and makes a failed transition invalid. `FailurePlan` exists only for a typed failure slot.
`Handled` routes the exact resolved source slot into one ordinary `Pure + Never` handler state and
exhaustively selects either recovery of the protected output or the current scope's exact failure
channel. On the protected failure's own normal-control continuation, the pre-handler and route
blocks cannot directly bypass the designated handler or ignore its exact output. A distinct
failure newly committed by an intervening pre-handler binding causally supersedes the protected
failure and follows only its own exact plan; this is not a direct escape by the protected source.
`.or_default()` constructs only `DefaultPropagation`; its exact handler variant-payload slot is
the exact failure tail. An explicitly authored handler constructs `CustomRecovery` and cannot
masquerade as the default route.

`Propagate` exists only inside a fresh expansion fragment. A non-cloneable token names that exact
enclosing boundary, and the boundary slot uses the mapping chain's exact target as its source.
The fields cannot name different same-typed boundaries or lexical regions. A source therefore
cannot target a sibling, ancestor, unrelated fragment, lane, or operation root. A zero-link
propagation is legal only when source and boundary contracts are exactly equal. Otherwise every
mapping link is an ordinary certified `Pure + Never` state binding in the token's exact active
lexical region, under the same plan identity, with exact adjacent input/output contract and
producer equality. The `CertifiedProgram` contains only its slots and binding identity; the fold later
requires the corresponding committed transitions, resolves their content references, and derives
the exact fragment-boundary value. Finite nested propagation must end in exactly one handled
call-site plan.

A scope default is not one polymorphic implementation. It is a finite exact table:

```text
LexicalFailureMap<ScopeFailure> =
    match enclosing_owned_scope.failure_contract {
        Never =>
            EmptyExactMap
        Typed(ScopeFailureContract) =>
            ExactFiniteMap {
                SourceFailureContract
                    -> RegisteredPureNeverMapper<
                           SourceFailure,
                           DefaultScopeFailureRoute<
                               ScopeFailure,
                               ScopeFailureContract,
                           >,
                       >
            }
    }
```

`.or_default()` resolves only when one exact table entry matches the complete source
`RetainedValueContract`. Sources with the same exact contract may share an identity mapper.
Heterogeneous sources use different mapper states. A child or expansion maps its internal leaf
failures into its closed boundary before the parent sees it. A scope with
`FailureContract::Never` has no table and must explicitly recover every owned fallible source.

An outcome-affecting wrapper must preserve the protected call's exact success and failure
boundary. A denial therefore requires an exact registered mapper into that typed failure
contract. A denying wrapper cannot wrap a `Never` boundary. A non-denying observer may wrap
`Never` only when every support state is also `Never`; otherwise the policy belongs at a fallible
enclosing boundary. Certification rejects rather than silently widening an operation's declared
failure sum.

Match tags and stable arm labels are unique and exhaustive for the certified closed sum. Stable
fan-out keys are unique in their group. Dense ordinals are derived from retained declaration
order and govern execution and join order; authors do not supply them. Stable labels and keys
anchor semantic identity and are not replaced by ordinals.

This algebra is a structured tree. Sub-blocks cannot name arbitrary program counters, alias a
continuation, jump into another block, construct the operation outcome from a lane, or expose an
inactive branch value. An implementation may derive an indexed occurrence table as a
process-local cache, but it is neither admitted nor hashed separately.

Every syntactic path either produces the exact current block's normal result or its exact lexical
failure. A committed state transition alone constructs `StateOutcome`; a lane tail constructs one
nominal `LaneOutcome`; and the containing run root constructs one nominal `OperationOutcome`.
A fragment directly rebinds its body's two structural channels to its exact boundary references;
it constructs no outcome value. There are no result instructions, dependency skips,
required-success sets, outcome nodes, or implicit terminal nodes.

### Operation DSL

The public authoring surface should make order and fan-out visually explicit while keeping
sequence implicit:

```rust
operation::<Snapshot, SnapshotFailure>("snapshot", |op| {
    op.failure_map::<ReadAnchorFailure, MapReadAnchorFailure>();
    op.failure_map::<ReadBalanceFailure, MapReadBalanceFailure>();

    let anchor = op
        .read::<ReadAnchor>("anchor", input)
        .or_default();

    let balances = op.fan_out::<MAX_ASSETS>(
        "balances",
        assets,
        |lane, asset| {
            lane.failure_map::<
                ReadBalanceFailure,
                MapBalanceFailureToLane,
            >();

            lane.read::<ReadBalance>(
                "read",
                (anchor, asset),
            )
            .or_default()
        },
    );

    let snapshot = op
        .pure::<Aggregate>("aggregate", balances)
        .on_failure::<HandleAggregationFailure>();

    op.succeed(snapshot)
});
```

This is illustrative, not a frozen Rust API. The required properties are:

- builder handles are private or affine enough to prevent forward references and core
  duplication;
- declaration order is retained exactly;
- stable labels are explicit and unique in their lexical scope;
- an authored `OperationCall` has a typed success/failure boundary and cannot survive expansion;
- `or_default` is permitted only when a compatible lexical default handler exists;
- a lane declares its own mapper unless its failure contract is exactly compatible with an
  explicitly inherited default;
- branch arms are exhaustive;
- continuing arms produce compatible types;
- lane values cannot cross lane boundaries; and
- a fan-out lane can produce only its declared lane outcome, never the containing operation's
  outcome.

`succeed(value)` selects the root block's normal expression, and any corresponding
`fail(failure)` helper selects its lexical failure channel. The fold later wraps the selected
channel in the nominal `OperationOutcome`; neither helper appends a binding or instruction. A
builder must reject a second root completion choice and any path for which neither channel is
well-typed.

More complex predicates are computed by an ordinary `Pure` state into a closed enum and then
matched. Closed sums use the kernel-owned canonical tag encoding frozen in the
`CertifiedProgram`.
Runtime and store never execute an unrecorded branch predicate or discriminator callback.

### Pure expansion

Capability lowering consumes one abstract semantic call token and replaces it with a structured
fragment having the same external boundary:

```text
Lower<SemanticCall<Input, Output, Failure>>
    -> Fragment<Input, Output, Failure>
```

A policy wrapper receives the already-lowered boundary and one affine `proceed` capability:

```text
Around<Boundary<Input, Output, Failure>>:
    Proceed<Boundary> -> Fragment<Input, Output, Failure>
```

`proceed` occurs structurally once. A precondition may choose a branch that does not execute it,
but no expansion can clone it, execute it twice, move it across sibling declarations, or capture
it inside fan-out.

A representative wrapper normalizes to:

```text
outer.pre
  inner.pre
    protected
  inner.after_success | inner.after_domain_failure
outer.after_success | outer.after_domain_failure
```

Profile order is outer-to-inner on entry and reverses on exit.
Failure-post fragments run before the enclosing lexical failure handler and must preserve or
explicitly map the wrapped call's declared failure boundary. Success-post fragments likewise
produce the wrapped call's declared success boundary.

The protected failure propagates affinely through the `FragmentBinding` as part of the same
failure continuation. Certification inserts no second inner handler for that propagated boundary.
States introduced inside the fragment retain their own lexical failure continuations. If a
failure-post state itself fails, that new failure follows the post-state's continuation; if it
settles successfully, the protected failure proceeds to its one call-site handler.

There is no implicit `finally`. Nothing can promise a post-state after process loss or while an
effect remains possible-entry ambiguous.

Expansion policies may not:

- inspect network, filesystem, clock, environment, store, Runtime, current history, adapter, or
  transport;
- reorder or remove sibling operation declarations;
- create cross-slot dependencies or use non-dominating values;
- introduce loops, jumps, detached work, or unbounded fan-out;
- hide an `Effect` inside fan-out;
- choose hidden, dynamic, or open-ended physical scheduling, retry, polling, backoff, or failover;
- construct live capability authority; or
- retain opaque executable callbacks in the certified program.

An expander may insert a finite, statically bounded sequence of distinct semantic retry, polling,
replacement, or fallback occurrences. Their requests, branches, order, and bound are visible in
the expanded program. No adapter or Runtime loop chooses additional occurrences.

The frozen expansion pipeline and eligibility matrix is:

| Phase | Eligible input | May emit | Later eligibility |
| --- | --- | --- | --- |
| Child substitution | Authored `OperationCall` | Inlined authored blocks and calls | Capability lowering and policy wrapping |
| Capability lowering | Authored or inlined abstract semantic call token with one exact registered requirement | One boundary-preserving fragment that consumes and replaces the token with executable support states | The preserved lowered boundary may receive policies; support states do not receive child/capability/policy expansion |
| Policy wrapping | Eligible lowered semantic boundaries under the exact admitted profile | Bounded pre/post states and `Match` around one affine `proceed` boundary | No policy applies to its own or another policy's support states |
| Failure completion | Every remaining typed-failure state or fragment boundary, including support states | One exact registered `Pure + Never` mapper/handler and its exhaustive route | Handler states are final leaves |
| Normalization and certification | Fully expanded structure | One canonical `CertifiedProgram` root whose exact closure binds the `ExpandedProgram`, profile, proofs, manifests, bounds, contracts, and implementation closure | Nothing |

Multiple policies compose in frozen profile order, outer-to-inner on entry and reverse on exit.
Every policy applies once to the preserved semantic call boundary, not recursively to states
injected by itself or another policy. Capability and policy fragments own the failure coverage,
security obligations, and qualification of their support states. When one policy's support truly
requires another policy, the registry must provide one explicit composite expansion; implicit
fixed-point expansion is forbidden.

The two affine transformations are distinct:

```text
Lower<SemanticCall<Input, Output, Failure>>:
    consume the abstract call token exactly once
      -> Fragment<Input, Output, Failure>

Around<Boundary<Input, Output, Failure>>:
    execute one affine proceed<Boundary> zero or one time
      -> Fragment<Input, Output, Failure>

composition:
    policy_0(policy_1(lower(call)))
```

Capability lowering may replace a high-level abstract call such as EVM submission; it does not
have to execute a nonexistent leaf state. Policy wrapping acts on the complete lowered semantic
boundary, so an outer security precondition runs before nonce reservation or any other support
state. A denying policy may choose not to invoke `proceed`; no policy may duplicate it.

An outcome-affecting policy must preserve the protected boundary exactly. A denial requires an
exact registered mapper into that boundary's typed failure contract; a denying policy is
ineligible for `Never`. A non-denying wrapper over `Never` is legal only when every support state
is also `Never`. The certifier rejects a policy that would silently widen an operation or fragment
failure contract.

Child substitution has one exact structural lowering:

```text
LowerChildBoundary<Output, Failure>:
    each child input root(child_root)
        <- FragmentInputSlot<
               child_root.Value,
               call_boundary.boundary_id,
               child_root.root_id,
               child_root.contract_ref,
               ExactDominatingCallerSlot<
                   child_root.Value,
                   call_boundary.boundary_id,
               >,
           >
    child normal channel(child_success_slot)
        -> ExactFragmentSuccessSlot<
               Output,
               call_boundary.success_contract,
               call_boundary.boundary_id,
               child_success_slot,
           >
    child lexical failure channel(child_failure_slot)
        -> ExactFragmentFailureSlot<
               Failure,
               call_boundary.failure_contract,
               call_boundary.boundary_id,
               child_failure_slot,
           >
```

Every child input root is substituted exactly once from its declared call-site input; the
certified fragment-input slot preserves caller provenance while rebinding lexical scope. The
certified fragment success and failure slots likewise retain the exact source slot, nominal
contract, child path, and call-site boundary identity. Only after an exact source value commits
does the fold resolve those recipes into `FragmentInputValueRef` and
`FragmentBoundaryValueRef`, preserving content and the complete source derivation. The inlined child
constructs neither a child `OperationOutcome` nor `StateOutcome`; those nominal forms belong only
to the containing run root and committed state transitions. Lowering creates no `State`, `Match`,
transition, child `RunClosed`, or executable occurrence. The parent consumes boundary success as
the call value and boundary failure through the call site's sealed failure plan. Certification
requires a total bijection over child/call-site input roots plus exact output and failure
contracts. Expansion preserves the child's lexical defaults internally and leaves no child call
in `ExpandedProgram`.
The certified failure slot is exactly
`ExactFragmentFailureSlot<Failure, call_boundary.failure_contract,
call_boundary.boundary_id, child_failure_slot>` and is eligible for that call site's
failure plan. Its symmetric success slot is
`ExactFragmentSuccessSlot<Output, call_boundary.success_contract,
call_boundary.boundary_id, child_success_slot>` and is not failure-plan eligible. At
execution, the fold resolves each only from its exact certified source and boundary role.

A policy never reapplies to states it injects, and injected support is never an implicit target
for a later policy. Expansion dependencies are acyclic and subject to hard fragment-depth,
occurrence, branch, fan-out-depth, lane, and total-expanded-program bounds.

Coverage is one closed structural proof:

```text
PolicyContract {
    policy_ref,
    exact_eligible_semantic_boundary_contracts,
}

CoverageProof =
    ordered [
        (
            semantic_boundary_id,
            required_profile_ordinal,
            exact_policy_ref,
        )
    ]
```

For every admitted semantic boundary and every policy required by that entry point's exact profile,
the proof has exactly one entry if and only if the boundary contract is eligible. Order matches
the profile and wrapper nesting. Injected support states do not appear as independent coverage
targets; their capability/policy expansion manifest owns them, or one explicit composite policy
does. Certification rejects missing, duplicate, foreign, reordered, or ineligible entries.

The qualified entry-point admission policy selects the trusted certified-program contract,
certification predicate set, required profile, exact policy versions, and coverage obligations.
Certification proves that the expanded program matches them; admission resolves the policy from
the qualified entry-point identity and rejects a weaker, stale, or caller-substituted policy,
predicate set, proof, or profile. An empty profile is legal exactly when the qualified entry point
requires an empty profile. The registered semantic state/capability contract selects capability
expansion. Runtime never discovers topology by inspecting which adapter or transport happens to
implement an operation.

### Failure handlers and custom recovery

After state settlement:

```text
StateOutcome<LexicalValueRef<Output>, LexicalValueRef<Failure>>
    Success(output) -> success continuation
    Failure(failure) -> exact structural failure continuation
                         -> failure-post states, if any
                         -> affine fragment boundary, if wrapped
                         -> designated failure handler
```

This split is part of `StateBinding` normalization, not an author-visible `Match`. The protected
failure has no raw arm that can directly bypass its certified normal-control continuation. If an
intervening failure-post state fails, its newly committed typed failure causally supersedes the
protected failure and follows that state's own exact continuation.

The default-handler registration is:

```text
LexicalFailureMap<ScopeFailure> {
    when enclosing_owned_scope.failure_contract
        == FailureContract::Typed(ScopeFailureContract),
    exact entries:
        SourceFailureRetainedValueContract
          -> RegisteredMapper {
                 Kind = Pure,
                 Input = ExactResultSlotOf<before_handler>,
                 Output = DefaultScopeFailureRoute<
                     ScopeFailure,
                     ScopeFailureContract,
                 >,
                 FailureContract = Never,
             },
}
```

The source is a certified `StateBinding` or `FragmentBoundary`. One monomorphic handler does not
pretend to consume heterogeneous failures. Sources with identical complete contracts may share a
mapper; distinct source contracts require distinct entries. `.or_default()` is admitted only
when exactly one entry matches.

`DefaultScopeFailureRoute<ScopeFailure, ScopeFailureContract>` is the default handler's registered
`ClosedHandlerRoute`: it has one `Propagate(ScopeFailure)` tag.
`DefaultScopeFailureContinuation` exposes the exact `ExactVariantPayloadSlot` derived from that
handler output. The enclosing scope's exact typed failure contract, the registered `Propagate`
payload entry, the derived payload slot, and the failure tail all carry the same nominal contract
reference. The continuation permits no intervening binding and requires the scope-failure tail
to use that slot. It cannot replace the payload with another dominating same-typed value or with
byte-identical payload material registered under a different contract. The route is unavailable
when `ScopeFailure == kernel::Never`. A custom handler may register a richer closed route, but its
exact output remains the sole selector of the certified exhaustive table; another value cannot
select an arm or bypass the handler output.

For an operation scope, the normal default expansion is:

```text
StateOutcome::Failure(f):
    route = ExactRegisteredMapperFor(f)(f)
    match route {
        Propagate(operation_failure):
            exit the operation's lexical failure channel
    }
```

For a fan-out lane scope, the same shape produces
`LaneOutcome::Failure(LaneFailure)` as the lane result instead of producing the containing
operation's outcome. A custom handler may instead produce a closed scope-defined route enum
followed by an exhaustive `Match`. IO recovery, fallback, compensation, or a changed external
request appears as ordinary states in the selected branch. The handler itself neither performs IO
nor returns a generic `Retry` command.

Defaults are lexical:

- an operation owns a closed failure sum and exact mapper table for calls it authors;
- a child operation owns its internal defaults;
- an expansion owns failures from every state it injects and maps them to its advertised boundary;
  and
- certification rejects every uncovered fallible state or fragment boundary.

The failed source transition, exact failure object, handler input, handler output, and subsequent
branch remain independently visible in append-only history. Recovery never erases the original
failure.

### Fan-out

The initial contract is:

```text
FanOut<MAX> {
    ordered lanes fixed before admission,
    each lane: bounded LaneBlock<
        LaneOutputContractRef,
        LaneFailureContractRef,
        LaneOutput,
        LaneFailure,
    >,
    transitive execution kinds: Pure | Read,
    certified nesting depth: at most 2,
    join: collect all in declaration order,
}
```

Rules:

- lane cardinality is non-zero and may derive from canonical planning input but not
  runtime-discovered values;
- lane keys are unique and stable;
- lanes capture only immutable values that dominate the fan-out;
- lanes cannot reference each other;
- lanes may contain bounded fan-out only while the certified remaining depth is positive;
- the initial maximum fan-out nesting depth is two, and global occurrence/lane bounds cover the
  complete nested tree;
- lanes cannot produce the containing `OperationOutcome`;
- every lane produces exactly one typed outcome;
- each lane is its own lexical failure scope; a typed lane failure contract may use a compatible
  default pure handler to produce typed lane failure, while a `Never` lane has no failure slot,
  no default, and no constructible `LaneOutcome::Failure`; it constructs only lane success and
  explicitly recovers every fallible source;
- any custom lane recovery remains transitively `Pure` or `Read`;
- the join waits for every lane; and
- the result vector uses declared lane order, never completion order.

An outer `Pure` state may summarize lane outcomes and select a normal operation branch. This avoids
fail-fast cancellation, incomplete access histories, and nondeterministic “first failure” meaning.

Runtime chooses the lexicographically lowest actionable lane path by nested declaration ordinal.
A concurrent driver may authorize a different lane path only after every earlier path is durably
waiting on a read observation or complete. Exact-head CAS prevents two workers from authorizing
the same lane state at the same cursor. The store—not Runtime—derives and enforces the exact
minimum actionable path for every semantic action. An observation may commit for any exact
outstanding authorization because external completions need not arrive in lane order; its lane
then becomes actionable for declaration-ordered settlement.

### Certified program and folded cursor

The callback-free store fold derives one recursive cursor:

```text
StateLeaf =
    Ready
  | Authorized(AccessKind, AccessAttemptId)
  | ObservedForSettlement(AccessAttemptId, ObservationRef)
  | Refreshable(NextAttemptOrdinal, PublicLineageHeadRef)
  | EntryUnknown(AccessAttemptId)
  | BlockedIntegrity(IntegrityObservationRef)

LaneCursor =
    AtState {
        occurrence_path,
        leaf: StateLeaf,
    }
  | InFanOut {
        group_path,
        declaration_ordered_lane_states:
            [LaneCursor],
    }
  | Completed(LaneOutcomeRef<LaneOutput, LaneFailure>)

VerifiedProgramState {
    cursor:
        AtState {
            occurrence_path,
            leaf: StateLeaf,
        }
      | InFanOut {
            group_path,
            declaration_ordered_lane_states:
                [LaneCursor],
        }
      | Closed {
            outcome_ref:
                ContentRef<
                    OperationOutcome<
                        LexicalValueRef<Output>,
                        LexicalValueRef<Failure>,
                    >,
                >,
        },

    live_lexical_bindings,
    journal_head,
    semantic_head,
    semantic_state_digest,
}
```

After admission or a semantic transition, the fold normalizes through sequence boundaries,
`Match`, fan-out entry/join, and typed lexical results until it reaches the next executable state
occurrence or the root `OperationOutcome`.

No mutable cursor/status row is semantic authority. A backend may materialize an index only when
it is verified against the authoritative prefix and exact head. A process-local occurrence or
access lookup table is only a cache over this cursor and cannot become a second phase map.

`BlockedIntegrity` is fold-derived only from a committed
`ExternalAccessObserved::IntegrityFault`; its observation reference is the durable cause. No sixth
record family records a Runtime diagnostic. A `Pure` callback or codec/contract violation before a
transition leaves the prior fold cursor unchanged and returns a repeatable attributed component
fault. `settle -> InvalidEvidence` leaves the cursor at its already committed
`ObservedForSettlement` reference and deterministically reports that fault again. An invalid
append candidate is rejected without changing history. None of those detected faults can advance
the program, but they do not pretend that an unrecorded diagnostic is a durable semantic leaf.

### Runtime action derivation

Runtime receives a sealed verified view and derives only:

```text
CommitLocalState
AuthorizeCurrentAccess
SettleCurrentObservation
Closed
Waiting
BlockedIntegrity
```

The store computes one closed frontier algebra:

| Cursor leaf | Frontier |
| --- | --- |
| `Ready<Pure>` | `Actions[(path, CommitLocalState)]` |
| `Ready<Read | Effect>` | `Actions[(path, AuthorizeCurrentAccess)]` |
| `ObservedForSettlement` | `Actions[(path, SettleCurrentObservation)]` |
| `Refreshable<Effect>` | `Actions[(path, AuthorizeCurrentAccess)]` |
| `Authorized<Read>` | `WaitingReads` |
| `Authorized<Effect>` or `EntryUnknown` | `Barrier(PossibleEntry)` |
| `BlockedIntegrity` | `Barrier(Integrity)` |
| `Completed` | `Complete` |

`Refreshable` for a Read or any Effect leaf inside fan-out is invalid before frontier derivation.
For `InFanOut`, the fold scans lanes in declaration order:

1. `Complete` and `WaitingReads` lanes permit scanning the next lane.
2. The first lane returning `Actions` contributes its recursively derived action list and stops
   the outer scan; no later outer lane is eligible yet.
3. The first `Barrier` stops with that barrier; later lanes are not actionable.
4. If every lane is `Complete`, callback-free normalization constructs the join rather than
   exposing an action.
5. If at least one lane is `WaitingReads` and every other lane is `Complete` or
   `WaitingReads`, the group is `WaitingReads`.

Nested fan-out applies the same recursion, so its returned action paths already obey all inner
lane gates. `actionable_paths(cursor)` is exactly the ordered paths in the resulting `Actions`;
for any other frontier it is empty. The semantic append path must equal the lexicographic minimum
of that exact list. An observation append is not a semantic action: it may target any exact
outstanding authorization and then causes frontier derivation to run again. This table is shared
by every backend, Runtime view, and recorded replay.

Inside fan-out, the current access names one eligible `Read` state occurrence within a lane; a lane
or fan-out group itself is never an access target. There is no global node scan,
authorization-count spreading, alternative-source readiness, dependency skip, or action-family
fairness rule.

For authorization and state settlement, the store accepts only:

```text
candidate.occurrence_path
    == min_lexicographic(actionable_paths(verified.cursor))
```

An `Authorized` read is waiting rather than actionable, which permits a later lane to receive its
authorization. Observation append is the sole exception: it may target any exact outstanding
authorization, after which the ordinary minimum-path rule governs settlement.

One `drive_once` performs at most one semantic transition or one audited access operation. Pure
control normalization is folded into the transition commit that produced its discriminant; it
does not require fake state or control records.

When a semantic transition makes the root `OperationOutcome` derivable, `RunClosed` is committed
in the same atomic append with a reference to that outcome. If the root outcome is derivable during initial
normalization without a state transition, admission atomically appends `RunAdmitted` and
`RunClosed`. Admission-only normalization may traverse a root expression, admission-root `Match`,
or a non-empty state-free fan-out whose lane tails use only admission roots; there is no special
empty-join path. The outcome expression is never a separately executable cursor position or
independently appended control record. The same atomic append admits and binds exactly one root
content-addressed `OperationOutcome` object. When normalization traverses a state-free fan-out, it
also binds that root's complete structural object closure—every required lane wrapper, nested join,
and lexical derivation—in the same atomic append. Those support objects do not become additional
root outcomes. The required `RunClosed` contains only the one root outcome reference:

```text
RunClosed {
    outcome_ref:
        ContentRef<
            OperationOutcome<
                LexicalValueRef<Output>,
                LexicalValueRef<Failure>,
            >,
        >,
}
```

There is no second inline outcome encoding. Closure is illegal while the current occurrence or
any entered fan-out lane has unresolved access. No semantic or audit record is accepted after
closure.

The referenced object retains the nominal `OperationOutcome` variant and the exact active
`LexicalValueRef` from the selected path, including admission-root or structural derivation when
applicable. `outcome_ref` is a content/object reference, not an occurrence identity. The store
rejects a missing or unbound object, a second inline encoding, a wrong variant, contract, source
derivation, inactive-arm reference, or standalone/delayed closure.

## State And Access Algebra

### One closed state algebra

```text
StateExecution =
    Pure(PureStateContract)
  | Read(ReadStateContract)
  | Effect(EffectStateContract)
```

Conceptually:

```text
Pure:
    apply(StateFrame)
      -> ProposedStateOutcome<Output, Failure>

Read:
    request(StateFrame)
      -> ReadRequest

    settle(
        StateFrame,
        CommittedObservationView<
            Returned(ReadResponse)
          | SafeFailure(ReadSafeFailure)
        >
    ) -> ProposedStateOutcome<Output, Failure>
       | InvalidEvidence

Effect:
    request(StateFrame)
      -> EffectRequest

    settle(
        StateFrame,
        CommittedObservationView<
            Returned(EffectResponse)
          | SafeFailure(EffectSafeFailure)
        >
    ) -> ProposedStateOutcome<Output, Failure>
       | InvalidEvidence
```

The callback result is uncommitted proposal material. The exact-head append validates its variant
against `FailureContract`, persists the selected canonical value and structural references, and
constructs the nominal `StateOutcome` only as part of the accepted transition. No callback,
adapter, or rejected append can construct committed state-outcome authority.

Request authorship is total over the certified frame. A state that may terminate without external
IO is preceded by an explicit `Pure` decision and `Match`, normally injected by expansion.
Conditional progression belongs in the structured operation, not in a state-selected next
action.

There is no generic `BlockedUnresolved` state result. A bounded domain policy that exhausts its
evidence returns a typed failure or closed enum and follows its explicit operation branch.
Physical store inability, integrity failure, and possible-entry ambiguity may still make Runtime
report `Waiting` or `BlockedIntegrity`; those are not domain outcomes.

Every semantic retry, poll, fallback, replacement, or changed request is a distinct explicit state
occurrence inserted by operation authoring or pure expansion.

### Private access protocol

The only live sequence is:

```text
prepare<K>(QualifiedPhysicalBinding<K>)
  -> Prepared<K>

authorize(Prepared<K>)
  -> append ExternalAccessAuthorized
  -> CommittedAccessAuthorization<K>
  -> Authorized<K>

invoke(Authorized<K>)
  -> AccessCompletion<K>
  -> PendingObservation<K>

commit_observation(PendingObservation<K>)
  -> append or resolve ExternalAccessObserved
  -> CommittedObservation<K>
```

Constructors and fields of authority-bearing types are private.

`Authorized<K>` owns the exact prepared request and binding. Passing a second request beside the
authority is forbidden.

The runtime-facing completions are exhaustive:

```text
AccessCompletion<Read> =
    Returned(ReadResponse)
  | SafeFailure(ReadSafeFailure)
  | IntegrityFault(AccessFaultCode)

AccessCompletion<Effect> =
    Returned(EffectResponse)
  | SafeFailure(EffectSafeFailure)
  | SupersededBeforeEntry(PhysicalBindingRefreshEvidence)
  | EntryUnknown(AccessFaultCode)
  | IntegrityFault(AccessFaultCode)
```

`SupersededBeforeEntry` is available only to qualified resource operations whose authority can
atomically prove that the protected semantic mutation or exclusive capability consumption was not
applied. It is physical control evidence and is never state-consumable. Its evidence contains an
opaque secret-free public lineage-head reference under a manifest-declared generic
physical-binding contract. The store fold retains that reference in `Refreshable` as a monotonic
lower bound; a separately assembled current worker supplies a binding whose public certificate is
that head or a verifier-proven non-rollback descendant in the same admitted stable lineage.
Runtime interprets no resource fields. The private writer credential is never returned or
persisted.

`EntryUnknown` is audit evidence that the authorized call's completion is unavailable and target
entry cannot be excluded. It leaves the occurrence current and permits no new authorization for
that occurrence.

An unmatched read authorization remains folded as `Authorized<Read>`. A recovery view with no
corresponding live affine token reports `ReadCompletionUnknown`, not `EntryUnknown`. The
selected initial policy deliberately defines no same-occurrence successor. It must not be treated
as an Effect ambiguity, silently reauthorized, or synthesized into a domain result.

`IntegrityFault` is audit-only and blocks. It never becomes state failure.

There is no outer `Result` after authority consumption. Internal code may use `Result`, but the
private invocation wrapper must totalize every normal return into one bounded canonical completion.

Rust cannot guarantee observation after process death. The enforced type property is narrower:

```text
no constructible normal-return path skips exact observation commit
```

### Prepared access

Before authorization, Runtime and store preparation freeze:

- run, occurrence, semantic call, and current semantic head;
- structured cursor and fan-out lane when applicable;
- certified execution and access kind;
- operation identity and implementation binding;
- immutable typed request and canonical bytes;
- request, response, safe-failure, and fault contracts;
- the optional certified protected-non-application refresh contract and its stable lineage;
- stable qualified routing, signer, and resource-lineage references where applicable;
- the current secret-free physical binding reference;
- a fold-derived access-attempt ordinal and stable attempt identity; and
- exact producer-bound input provenance.

Any failure here happens before live authority and appends no authorization.

There is no raw-reference Runtime preparation API. Runtime-private preparation accepts the sealed
qualified binding and retains its private invoker handle. It sends only the immutable secret-free
public certificate and producer-free request material to the store. The store verifies that
certificate against the admitted stable lineage and implementation binding and authors the
attempt's public binding reference itself. A positive append proof lets Runtime combine the
retained handle with `CommittedAccessAuthorization<K>` to construct `Authorized<K>`; an ambiguous or
rejected append cannot do so. Replay revalidates the certificate relation without ever obtaining
the private invoker handle.

A generic `AccessAttemptId` is derived by the kernel from the exact run, occurrence, attempt
ordinal, structured cursor/semantic-head anchor, operation, immutable implementation binding,
current physical binding reference, and request digest. The first attempt has ordinal zero.

A definitely rejected stale authorization candidate consumes no ordinal. Runtime reloads and may
prepare that ordinal again only if the exact occurrence remains the minimum actionable path and
the fold still derives the same ordinal. Authorization or transition candidates are never blindly
rebased. Any changed certified cursor or semantic-head anchor—including a fan-out sibling becoming
observed—requires rebuilding the candidate and its identity; if only the predecessor envelope
changed while the certified anchor is provably identical, only that envelope is rebuilt.

The distinct rebase exception is an already linked `PendingObservation`: its authorization,
attempt identity, request, and completion are immutable. If its append loses an exact-head race to
unrelated history, Runtime may rebuild only the predecessor-bound envelope after the store proves
that exact authorization is still outstanding; it never reinvokes the target. An ambiguous
acknowledgement must resolve the unchanged original append identity before any rebuild. Once an
authorization positively exists, its ordinal is consumed and its identity/content are immutable.

Domain resource identities may additionally use a separately certified permanent semantic intent
identity, but Runtime and the run-history store never inspect domain-specific fields.

### Attempt typestate

The callback-free fold derives one of two sealed states for the current access occurrence:

```text
ReadAttemptState =
    Ready<Read, AttemptOrdinal>
  | Authorized<Read, AccessAttemptId>
  | ObservedForSettlement<Read, AccessAttemptId, ObservationRef>
  | BlockedIntegrity<IntegrityObservationRef>

EffectAttemptState =
    Ready<Effect, AttemptOrdinal>
  | Authorized<Effect, AccessAttemptId>
  | ObservedForSettlement<Effect, AccessAttemptId, ObservationRef>
  | Refreshable<Effect, NextAttemptOrdinal, PublicLineageHeadRef>
  | EntryUnknown<AccessAttemptId>
  | BlockedIntegrity<IntegrityObservationRef>
```

Only `Ready` and `Refreshable` expose the private preparation transition. Only a committed
`SupersededBeforeEntry` observation can create `Refreshable`, and the fold increments its ordinal
exactly once. An unmatched `Authorized<Read>` is reported by the recovery view as
`ReadCompletionUnknown`; it is not a separately persisted transition. `Authorized` and
`EntryUnknown` expose no constructor for another authorization. This prevents overlapping attempts
by construction in process and by candidate validation for hostile persisted bytes.

`Refreshable` and `EntryUnknown` are Effect-only constructors. Certification includes
`SupersededBeforeEntry` in an Effect completion contract only when that contract declares the
generic protected-non-application refresh capability and one admitted stable physical lineage.
An ordinary Effect and every Read reject that outcome variant.

`Refreshable` does not contain a writer credential. A currently qualified worker must present a
new sealed `QualifiedPhysicalBinding<K>`. The purpose-limited certificate verifier proves that its
public head is the recorded `Refreshable` head or a monotonic descendant in the same admitted
stable lineage, rejecting rollback and sibling lineages, before the store prepares the next
attempt. A further rotation racing that attempt returns another qualified
`SupersededBeforeEntry`; it cannot strand or let an old worker self-upgrade. An old worker remains
fenced.

### Observation persistence

`PendingObservation<K>` owns stable logical material independently of a predecessor-bound physical
append candidate.

Runtime:

1. resolves the authorization's observation logical key;
2. accepts an existing observation only when its canonical content is identical;
3. prepares an append against the current verified journal head;
4. rebases after a definite stale-head result without reinvoking;
5. resolves acknowledgement ambiguity using the unchanged physical append attempt; and
6. returns a committed proof only after positive append or exact identical resolution.

While the task lives, store unavailability does not turn pending material into normal success.
Task or process loss may discard pending material and leave the authorization unmatched.

### Attempt and ambiguity rules

For every current occurrence:

- one newly committed authorization mints one live invocation;
- a positive observation resolves that authorization;
- a returned `SafeFailure` is interpreted by the state and never enables reauthorization;
- a returned `SupersededBeforeEntry` resolves the physical attempt but not the semantic
  occurrence and creates only the next fold-derived `Refreshable` ordinal;
- a returned `EntryUnknown` parks the occurrence;
- an unmatched effect authorization remains `Authorized<Effect>` and is reported on recovery as
  possible-entry ambiguous;
- an unmatched read remains `Authorized<Read>` and is reported as `ReadCompletionUnknown`; and
- a later semantic request is always a different certified occurrence.

An ordinary failure handler cannot rewind the cursor or mint access authority.

The initial design performs no automatic same-occurrence Effect re-entry after an unmatched
authorization. A later read after a definite result is a distinct explicit read occurrence.
An unmatched read remains parked under the selected conservative initial policy; it is not
reauthorized merely because reads are non-mutating.
`EntryUnknown` cannot advance, close, or reach a later reconciliation state because it leaves the
ambiguous effect current. No manual mutation of history is defined. Any future reconciliation
must extend the same-occurrence access protocol under its own certified contract; it is not part
of this RFC. A process lease or elapsed time is never proof that the original effect was not
applied.

### Failure classification

| Evidence | State-consumable? | Required behavior |
| --- | --- | --- |
| `Returned(Response)` | Yes, through the state's pure settlement callback | Propose success, typed failure, or `InvalidEvidence`; only an accepted transition constructs committed outcome authority. |
| `SafeFailure` | Yes, through the state's pure settlement callback | A valid admitted instance proposes its qualified success or typed-failure disposition; only malformed/inconsistent evidence produces `InvalidEvidence`; never treat it as retry authority. |
| committed `StateOutcome::Failure(LexicalValueRef<S::Failure>)` | Yes, only by its exact `FailurePlanBound` mapper/handler input | Follow the one selected explicit/default failure plan. |
| `SupersededBeforeEntry` | No | Keep the same semantic occurrence current; fold to `Refreshable` with the next ordinal. |
| `EntryUnknown` or unmatched effect authorization | No | Keep the exact effect current with no legal successor in this RFC; a future same-occurrence protocol is required. |
| Unmatched read authorization | No | Keep `Authorized<Read>` in the fold and report `ReadCompletionUnknown`; the initial contract has no same-occurrence successor. |
| Committed `ExternalAccessObserved::IntegrityFault` | No | Fold to `BlockedIntegrity(observation_ref)`; never invent domain failure. |
| `InvalidEvidence`, callback, codec, contract, or rejected-candidate fault detected without a new record | No | Leave the authoritative prefix/cursor unchanged, report the repeatable attributed fault, and append no diagnostic event. |
| Journal/store interruption | No | Resume the physical persistence protocol; do not fabricate an access result. |

Production qualification must inventory every expected definite operational disposition as a
typed response, state-facing `SafeFailure`, retryable physical-control outcome, possible-entry
ambiguity, or integrity fault. When the product expects the state to fail or recover, the
capability must expose either a reviewed typed response or a reviewed redaction-safe `SafeFailure`
that the state can map into its typed outcome. Leaving such an expected definite condition as
audit-only non-domain evidence is an incomplete product contract.

For every capability/state pairing, qualification proves an exhaustive mapping for each admitted
`SafeFailure` variant. A state with `FailureContract::Never` may admit a variant only
when all valid evidence maps to success; if the expected disposition is negative, the state must
declare a real typed failure contract or the pairing is rejected.

For an `Effect`, `SafeFailure` must be a definite semantic disposition under the certified
operation contract. Evidence that leaves target application unknown is `EntryUnknown`, never a
`SafeFailure` and never a typed state failure. Qualification tests fallible `Read` and `Effect`
pairings separately because only the latter has this target-entry distinction.

## Run History And Store Enforcement

### Five record families

The append-only algebra remains:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

These are typed events in the existing `run:*` stream family. Past records are never mutated or
reinterpreted.

“Five record families” means every durable run-history entry is exactly one of those five event
shapes; it does not imply five mutable tables or five writers. “Exact-head atomic append” means a
backend locks or compare-and-swaps against the exact current predecessor, validates the complete
candidate against the authoritative prefix, and commits the whole candidate—including any
required object/fact bindings and adjacent closure record—or commits none of it.

Authorization and observation are audit records, not fake semantic transitions. They advance the
journal head. `RunAdmitted` initializes:

```text
SemanticHead::Genesis {
    admission_ref,
    genesis_semantic_state_digest,
}
```

That genesis value anchors access before the first state transition.
`StateTransitionCommitted` advances the semantic head thereafter. No record is accepted after
`RunClosed`.

An atomic append candidate must contain the adjacent `RunAdmitted + RunClosed` pair if and only if
the root outcome is initially derivable. It must contain
`StateTransitionCommitted + RunClosed` if and only if that transition first makes the root outcome
derivable. Omitting, delaying, or prematurely adding `RunClosed` is illegal. The records remain
separately hashed members of the same five-family algebra; either required pair is all-or-nothing.

`RunAdmitted` binds:

- tenant, store, run, invocation, and entry-point identity;
- one `CertifiedProgramRef` plus its complete content-addressed component closure;
- the exact qualified entry-point admission-policy reference that trust-anchors the program
  contract, certification predicate set, expansion profile, policy versions, and coverage;
- audit-projected authored/expanded/profile/proof/manifest references that must equal the
  corresponding fields of that exact `CertifiedProgram`;
- configuration, context roots, initial values, and prior-run source manifests;
- immutable secret-free routing policy and stable resource-lineage references; and
- the canonical genesis semantic-state digest.

Rotating public physical-binding references are bound per access attempt under those stable
lineages. Private signer or writer credentials are assembly-only and are never admission data.

### One callback-free fold

One pure fold validates complete history, previews candidate successors, and supports recorded
replay. Runtime must not implement a competing semantic fold.

The store fold verifies:

- canonical encoding, domain-separated hashes, record and commit identities; structured JSON
  hashing retains the repository's JCS-style canonicalization contract and rejects floats;
- contiguous sequence and exact predecessor relation;
- per-append atomicity and object closure;
- content-addressed manifests, context snapshots, facts, outputs, and retained evidence;
- unique logical keys and exact-content idempotency;
- the one canonical `CertifiedProgramRef`, its complete component closure, exact audit
  projections, qualified entry-point admission-policy and predicate-set binding, included proof
  validity under that exact set, and process-qualified implementation membership;
- exact current cursor and execution kind;
- lexical value dominance and producer-bound references;
- branch selection from the kernel's canonical closed-sum tags and certified arm table;
- fan-out lane eligibility, completeness, and declared result order;
- access-attempt ordinal, physical-binding, authorization, observation, refresh, and settlement
  linkage;
- typed outputs, facts, and failures;
- before/after semantic digest;
- exact typed root outcome; and
- atomic closure.

The store is authoritative for persisted protocol shape, provenance, and successor legality. It
does not rerun arbitrary state callbacks during recorded verification and does not prove
mathematical domain truth. Exact reproduction may separately rerun deterministic expansion and
pure callbacks.

### Generic append-boundary rejection

Every backend rejects a candidate unless all of the following hold against the locked current
prefix:

- an authorization or transition names exactly the lexicographically minimum actionable state
  path derived from the recursive cursor, never merely any ready lane and never the lane or group
  itself;
- no earlier declaration on the exact certified lexical path was skipped;
- the occurrence has not already settled;
- the execution kind matches the certified state;
- authorization is for the current `Read` or `Effect`;
- no unresolved authorization for that occurrence already exists, except that a certified
  `Refreshable` state permits exactly its next attempt ordinal;
- an observation names the exact authorization and immutable request;
- an observation may name any exact outstanding authorization regardless of lane order, but it
  cannot settle or advance that lane until the minimum actionable-path rule selects it;
- every observation completion variant and schema belongs to the exact prepared and certified
  exhaustive `AccessCompletion<K>` contract for that capability/state pairing;
- only those exact `Returned` and `SafeFailure` variants can become state-consumable observation
  material; `SupersededBeforeEntry`, `EntryUnknown`, `IntegrityFault`, and foreign completion bytes
  cannot be relabelled, decoded, or settled through that channel;
- `SupersededBeforeEntry` appears only for an Effect whose certified completion and prepared
  access declare that generic refresh contract, and its evidence matches the admitted stable
  lineage and qualified public binding certificate;
- settlement consumes the exact compatible committed observation;
- a state success follows its structural success continuation;
- a state or fragment failure follows its sealed `FailurePlan`;
- a normally completing `Handled` pre-handler path can produce only its exact
  `ExactResultSlotOf<before_handler>`, and the handler's retained closed tag selects exactly one
  arm of its exhaustive certified continuation;
- a `Propagate` path can produce only its exact affine enclosing fragment-boundary result,
  preserves the lexical-region token and source-to-boundary failure provenance, and belongs to a
  finite chain ending in exactly one `Handled` call-site plan;
- a new failure from a failure-post state follows that post-state's own `FailurePlan`;
- every failure-path input names the exact failure-producing occurrence and contract;
- `Match` choice matches the retained canonical closed-sum tag;
- inactive-arm and non-dominating values are unusable;
- fan-out lanes are declared, unique, effect-free, and joined only when complete;
- fan-out output order is declaration order;
- outputs, failures, and facts use the exact occurrence and content contract;
- the exact root `OperationOutcome` is derivable before closure; and
- `RunClosed` is present if and only if that append first makes the exact root outcome derivable,
  inseparably from either admission or the responsible semantic transition.

Runtime supplies sealed callback results without caller-authored producer identities. The store
attaches the authoritative producer references and constructs input manifests, transition bodies,
binding deltas, digests, and optional closure. Persisted bytes remain hostile even when Rust
typestate made invalid construction difficult.

### Replay and projections

Recorded replay uses the same callback-free fold and proves what the run admitted, authorized,
observed, and committed. It invokes no adapter, transport, provider, signer, resource authority,
or mutable store path.

Trace, audit, export, CLI, and REST may expose reviewed identifiers, structural paths, state/access
kinds, secret-free physical-binding references, typed public failures, redaction-safe fault codes, and
whether a run is waiting on possible-entry ambiguity.

They never expose provider text, response bodies, endpoint URLs, credentials, raw SQL errors,
filesystem paths, signed payloads, or secret material.

### Prior-run fact selection is an ordinary read

A state that consumes prior-run facts declares an ordinary `Read` request containing its admitted
source manifest, selector contract, completeness mode, and bounds. The registered invoker is the
sealed purpose-limited RunHistory fact scanner rather than a general database adapter. It returns
one canonical typed response through the normal access bracket; it receives no append or generic
query authority.

Certification freezes the selector and source contracts. Authorization atomically captures the
current `TenantFactFrontier`; the store mints one affine scan permit; the sealed scanner proves
bounded completeness through that exact frontier; and the response carries the attestation
retained by observation persistence. The run-history fold validates the resulting observation and
producer references generically. The existing barrier/frontier/completeness semantics survive
behind the ordinary `Read` invoker. No `FactSelection` execution kind, alternate history protocol,
generic query authority, or ambient state callback is introduced.

## Expansion For Control, Security, And Telemetry

### Semantic injection

Expansion is the supported mechanism for reusable semantic composition:

- authorization and policy guards;
- provenance and configuration validation;
- nonce observation and reservation;
- reconciliation after a definite state-consumable result;
- deterministic transformations;
- success and domain-failure postconditions;
- compensation branches;
- security checks whose result changes whether work may proceed; and
- durable audit facts.

An injected IO operation is an ordinary `Read` or `Effect`. It receives authorization,
observation, typed failure handling, append-only history, and the same crash semantics as any
authored state.

A security pre-state cannot replace target-side authorization or cross-run resource fencing. A
post-state cannot undo an effect that already entered. No post-state executes while the protected
effect remains possible-entry ambiguous.

### Operational observation

Best-effort logs, metrics, and spans should observe redacted Runtime or committed-history events
outside semantic execution.

Making a logging sink an injected `Effect` would make collector availability part of run
correctness, add authorization and history volume, and create its own ambiguity and failure
handling. That representation is valid only when acknowledgement by the sink is genuinely part of
the operation's required semantics.

Authoritative audit data belongs in state facts, access records, and committed history. Operational
telemetry may be missing or duplicated and must never influence cursor derivation.

## Wallet Nonce Resource Authority

### Ownership and namespace

EVM sender nonces are shared by every actor capable of submitting for the same physical sender.
Declaration order within one run cannot serialize two runs.

The wallet-nonce authority owns one canonical physical namespace:

```text
ChainInstanceDeclaration {
    stable_chain_registry_id,
    never_reused_instance_namespace_id,
    chain_id,
    genesis_block_hash,
    immutable_finalized_fork_anchor {
        block_number,
        block_hash,
    },
}

QualifiedChainInstanceId =
    domain_separated_hash(
        "mfm.evm.qualified-chain-instance.v1",
        canonical_bytes(ChainInstanceDeclaration),
    )

EvmChainLineageId =
    domain_separated_hash(
        "mfm.evm.chain-lineage.v1",
        QualifiedChainInstanceId,
    )

WalletNonceDomain =
    EvmChainLineageId
  + sender identity
```

The EVM domain owns the declaration type and verification rules. A production deployment's
qualified chain-instance registry issues its immutable content reference, enforces a one-to-one
mapping from the inventoried physical chain to one never-reused namespace, and is part of
application qualification—not an RPC provider or Runtime. Assembly selects that reference;
`RunAdmitted` retains it, and the wallet-nonce authority binds its exact current-schema domain
activation record atomically with the first successful reservation and rejects rebinding.

Every admitted route generation proves membership by observing the declaration's chain ID,
genesis, and finalized fork anchor. The registry collapses redundant routes to one declaration
and assigns different namespaces to independently operated forks cloned from the same genesis.
Creating or recognizing a distinct fork always issues a new declaration; a retired namespace is
never reassigned. Production activation inventories the registry and rejects either two
declarations for one physical instance or one declaration assigned to two independent instances.
RPC identity alone is insufficient.

Tenant, wallet alias, route, and physical signer generation remain authorization and provenance
qualifiers, but they cannot partition the uniqueness namespace unless qualification proves a
one-to-one physical identity. A stable `SemanticSignerId` and signing-profile contract name the
public key/address and deterministic signing behavior required by the intent. Any admitted
physical signer generation must prove that exact semantic identity and behavior; generation
rotation cannot change transaction identity.

The physical `WalletNonceStoreLineageId` and monotonic `WriterEpoch` are distinct from
`EvmChainLineageId`. Qualified infrastructure permits exactly one actively fenced writable
physical incarnation of one store lineage at an epoch:

```text
WalletNonceStoreIncarnation {
    wallet_nonce_store_lineage_id,
    writer_epoch,
    physical_target_instance_id,
    non_exportable_target_public_key_ref,
    target_attestation_contract_ref,
}

QualifiedCurrentWalletNonceStoreIncarnation {
    public_incarnation_binding_ref,
    qualified_activation_registry_lineage_ref,
    qualified_target_fence_lineage_ref,
    private_target_bound_live_session,
}
```

The physical target owns the private key behind
`non_exportable_target_public_key_ref`; it is absent from database snapshots and backups, client
credentials, application processes, registry records, and every persisted MFM surface. The
deployment-owned `AuthoritativeWriterFence` issuer opens the sealed live session only for that
exact target key, physical database identity, store lineage, and writer epoch. The session and its
write capability are non-serializable and non-`Clone`; more importantly, the external fence makes
them non-transferable to a sibling target.

The deployment fence authority itself has one qualified non-rollback lineage with monotonically
irreversible target/session revocation and sibling-issuer exclusion. Retaining an old target key
cannot resurrect a revoked session. Every nonce-authority read atomically exercises its live
session while opening the exact database transaction/snapshot that returns status. A mutation
instead obtains a fresh, affine, non-replayable transaction permit bound to the exact database
session and transaction, then revalidates the public incarnation under the domain lock. Both proof
forms bind the qualified fence-authority lineage and its current irreversible head.

A static signature, copyable token, in-process self-assertion, copied database, lineage ID, public
attestation, client credential, registry row, replayed permit, rolled-back fence head, or sibling
fence issuer cannot return semantic status or accept a write without the target-held key and live
external fence. This is a qualified infrastructure guarantee with no production bypass, not a
property claimed from Rust type privacy alone.

Rotation, restore, and promotion carry a verified complete prefix containing every
domain-activation binding and its complete record closure, reservation, candidate activation,
completion, active-intent marker, permanent operation-key result, and high-water mark forward.
The qualified infrastructure control plane first permanently revokes old-target and sender-path
admission, then drains or aborts every old-epoch transaction, then captures and verifies the final
exact prefix and head after quiescence. The fence proof and complete-prefix proof bind the same
final old head. It hydrates and verifies the still-closed replacement from that prefix, publishes
the next epoch and target key by registry CAS, and only then opens the replacement. A pre-fence
snapshot, a snapshot without the complete final prefix, or a deployment unable to prove exclusive
target and sender-path fencing cannot become the same lineage; it requires a new sender/domain.

### Deployment cutover and virgin lineage

The current tree contains no legacy-schema reader, decoder, migration, or certifier. Git history
and an immutable export remain the audit archive; neither Runtime nor any current library,
adapter, application, binary, feature, or maintenance target can parse the retired schema.

Before activating the new wallet authority, deployment maintenance must stop old admissions,
drain every old run and allocation to a definite terminal disposition, fence every signer,
relayer, operator, stale deployment, and direct-submit path for the sender, rotate to a new
issuer/idempotency namespace, and retain the retired database only as offline opaque audit
material. The current deployment qualification registry admits the new
`WalletNonceStoreLineageId`, `WriterEpoch`, `WalletNonceDomain`, chain declaration, sender,
issuer-namespace contract, and exhaustive sender-path fence attestation as one immutable
secret-free activation record. It does not interpret, import, or attest individual retired
records.

One qualified wallet-activation registry authority owns a non-rollback registry lineage and
permanent, exact-key compare-and-append issuance. The PostgreSQL adapter's deployment-only
administrative plane implements it behind a role/pool that Runtime and normal application
assembly never receive. Its domain table permanently maps each `WalletNonceDomain` primary key to
exactly one `WalletNonceStoreLineageId` and one globally unique activation-record identity. Its
separate lineage table maps each `WalletNonceStoreLineageId` primary key to one monotonic current
`WalletNonceStoreIncarnation` head. Exact same domain issuance resolves the original proof; a
different lineage or activation identity conflicts. Exact same lineage-head publication resolves
the original proof; a sibling target or non-next epoch conflicts. One store lineage may
legitimately serve multiple domains, and all such domains share its one current incarnation, so
this is not a domain-to-lineage bijection.

Domain issuance and lineage promotion serialize on the exact lineage-table key. One
`issue_domain_activation` registry transaction locks that key, creates the initial lineage head
only when absent or validates the exact existing current head, compare-and-appends the domain row
and globally unique activation-record identity, and emits one composite proof binding the domain
row to the lineage head observed by that transaction. No crash or acknowledgement ambiguity can
leave only one table mutation visible; exact resolution checks both keys and returns the original
composite proof. A concurrent domain proposing a sibling initial target conflicts, and promotion
cannot interleave between the lineage-head validation and domain-row commit.

Promotion is the registry's only lineage-head advance operation. It requires the exact current
lineage head, the old-target fence proof, the post-quiescence complete-prefix proof for every
domain on that lineage bound to that same final head, the next writer epoch, the hydrated
replacement's target-key attestation, and proof that the replacement remains closed before it
atomically publishes the new lineage head. A crash before fencing completes leaves the old
registry head current and no replacement writable; a crash after fencing but before publish is
unavailable-safe and retries from the retained final head; exact replay after publish returns its
original proof. The replacement opens only after that proof. The old target and every old signer,
relayer, direct-submit path, session, and transaction can never regain write or submission
reachability. If any fencing, prefix, hydration, or promotion proof cannot be established, the
control plane fails closed and same-domain promotion is forbidden; a new sender/domain is
required.

Registry availability is required for deployment issuance and promotion, not for normal run reads
or mutations. Each normal authority access instead uses the sealed target-bound
current-incarnation session; a reusable public registry attestation alone is never read or write
authority, and querying the registry on each access would neither close the check/use race nor
replace the physical fence. The registry authority itself is admitted only on qualified
infrastructure providing the same single-writable-target, non-exportable-key, non-rollback
property.

The registry rejects a sibling or second activation for that domain even when the original
deployment is stopped or its writer is retired. Restore and promotion retain the same
store-lineage identity and verified complete durable prefix; they never create another virgin
lineage. Reinitialization under a new store lineage requires a new sender/domain.

The first domain on a new lineage qualifies the closed target and its target-bound capability
before the atomic composite issuance. A new domain on an existing lineage must instead verify and
bind the exact already-current lineage head; it cannot propose another target. Only after composite
issuance may the first local reservation bind that proof. A crash after registry issuance but
before the first reservation permanently pins the domain to that lineage; only exact issuance
replay on that lineage or a qualified promotion may proceed. A different lineage is never a
recovery path.

```text
WalletNonceDomainActivationRecord {
    activation_contract_ref,
    qualified_activation_registry_lineage_ref,
    wallet_nonce_store_lineage_id,
    initial_store_incarnation_ref,
    wallet_nonce_domain,
    qualified_chain_instance_declaration_ref,
    sender_identity,
    issuer_namespace_contract_ref,
    replay_exclusion_contract_ref,
    replay_exclusion_disposition:
        EveryPriorRequestReplayAndRetryIngressExcluded,
    qualified_finalized_sender_nonce_floor {
        finalized_block_number,
        finalized_block_hash,
        sender_nonce_at_finalized_block,
        qualified_observation_proof_ref,
    },
    exhaustive_sender_path_inventory_digest,
    exclusive_current_control:
        EveryPriorWriterSignerRelayerOperatorStaleDeployment
        AndDirectSubmitPathFenced,
    prior_effect_disposition:
        NoUnresolvedPossibleEntry,
    prior_resource_disposition:
        EveryPriorAllocationAndSubmittedCandidateTerminal,
    new_idempotency_epoch,
}

QualifiedWalletNonceDomainActivation {
    activation_record_ref,
    qualified_activation_registry_lineage_ref,
    composite_registry_issuance_proof_ref,
    verified_current_schema_record,
}
```

If operations cannot establish that every old effect is terminal, that replay/retry ingress is
excluded, or that every sender path is fenced, the new release must use a new sender/domain. There
is no high-water import escape. For a fully drained and fenced sender, the activation record binds
a qualified current-chain observation of its nonce at one canonical finalized block. The new
authority begins with a virgin retained lineage. Its first reservation still consumes a fresh
qualified `eth_getTransactionCount(sender, "pending")`, requires that value to equal the
activation record's finalized sender-nonce floor, and uses that pending value as the first
candidate. A lower value is a lagging or wrong-chain observation; a higher value contradicts the
qualified no-unresolved-effect and exclusive-control disposition. Either returns typed
`NonceLineageDiverged` without mutation. A newly generated sender follows the same rule with the
protocol initial nonce.

On the first reservation under the domain lock, the authority validates and permanently binds the
exact activation record before applying the ordinary virgin-lineage pending-floor algorithm.
Concurrent first reservations serialize under that same lock. A different activation record,
chain, sender, store lineage, issuer namespace, or fence attestation is an integrity conflict.
There is no separate bootstrap mutation, credential, role, result, or retry protocol.

The admitted `issuer_namespace_contract_ref` is immutable for the lifetime of that retained
wallet-authority history. Compatible implementation and physical-writer upgrades preserve the
same current schema, complete durable lineage, permanent operation-key results, and issuer
namespace. Any incompatible wallet-authority persisted-schema or issuer-namespace cutover
requires a new sender/domain. No same-sender legacy reader, dual schema, scalar import, or
bijective compatibility migration is retained.

### Operations

During normal run execution, the narrow authority exposes three idempotent mutations and one
purpose-limited read conceptually equivalent to:

```text
read_status(
    wallet_nonce_domain,
    target_bound_current_store_read_session,
    qualified_domain_activation,
    semantic_reservation_key,
    submission_intent_id,
    transaction_intent_digest,
    candidate_family_ref,
) -> WalletNonceStatus
   | SafeFailure
   | IntegrityFault

reserve(
    wallet_nonce_domain,
    target_bound_current_store_write_capability,
    qualified_domain_activation,
    semantic_reservation_key,
    submission_intent_id,
    transaction_intent_digest,
    transaction_intent_object_closure,
    candidate_family_ref,
    candidate_family_object_closure,
    observed_pending_floor,
) -> ReserveWalletNonceResponse
   | SupersededBeforeEntry(PublicLineageHeadRef)
   | SafeFailure
   | EntryUnknown
   | IntegrityFault

activate_candidate(
    wallet_nonce_domain,
    target_bound_current_store_write_capability,
    semantic_candidate_operation_key,
    next_candidate: ProducerBound<AttestedWalletCandidate>,
    activation_permit: CandidateActivationPermit,
) -> ActivateCandidateResponse
   | SupersededBeforeEntry(PublicLineageHeadRef)
   | SafeFailure
   | EntryUnknown
   | IntegrityFault

complete(
    wallet_nonce_domain,
    target_bound_current_store_write_capability,
    semantic_completion_key,
    provenance_verified_current_reservation,
    canonical_terminal_outcome,
    provenance_verified_terminal_witnesses,
) -> CompleteWalletNonceResponse
   | SupersededBeforeEntry(PublicLineageHeadRef)
   | SafeFailure
   | EntryUnknown
   | IntegrityFault

ReserveWalletNonceResponse =
    Reserved(ReservedWalletNonce)
  | NonceDomainBusy
  | NonceLineageDiverged
  | NonceCapacityExhausted

ActivateCandidateResponse =
    Activated(ActiveWalletCandidate)
  | CandidateProgressionConflict

CompleteWalletNonceResponse =
    Completed(CompletedWalletNonce)
```

`read_status` is an ordinary Runtime-authorized `Read`. The three mutations are
Runtime-authorized `Effect` accesses. The domain response sums are carried only inside generic
`Returned(Response)`; they are not extra access-completion variants. Internal `Applied` versus
`ExistingSame` status is not a semantic output.

The displayed target-bound session and write-capability arguments name assembly-private context
retained by the registered invoker. They are not state-authored or canonical request fields and
never enter programs, access records, histories, or replay.

`read_status` atomically exercises the sealed target-bound live session while opening one
transactionally consistent authority snapshot. It verifies the stable permanent domain binding
maps the requested domain to this store lineage and activation record before it may return even
`Absent`, then validates the expected intent identity and observes the reservation, complete
activated prefix, current candidate, and completion at one linearization point. `Absent` and
payload-free `Busy` are equally authoritative snapshot variants. A definitely stale or revoked
session returns a reviewed `SafeFailure`; a stale replica, retired writer lineage, torn prefix, or
malformed/unverified domain, chain, store, registry, or fence binding returns `IntegrityFault`.
Neither case returns semantic status.
An exact matching retained reservation or completion is resolved before considering another
intent's active marker, so a historical same-intent `Completed` or `Reserved` result cannot be
masked as foreign `Busy`.

For each mutation, the serialized resource transaction:

1. actively exercises the sealed target-bound session against the deployment
   `AuthoritativeWriterFence` and obtains a fresh non-replayable permit bound to this exact physical
   target, database session, transaction, store lineage, and writer epoch;
2. returns `SupersededBeforeEntry(public_lineage_head_ref)` without target entry when the fence
   proves that capability stale, or `EntryUnknown` when protected non-entry cannot be proved;
3. resolves an existing permanent semantic operation key on the admitted target first;
4. acquires the domain's serializing lock, revalidates the permit and exact current public
   incarnation under that lock, and re-resolves the key to close the concurrent-insert race;
5. validates the operation-specific permanent identity and semantic-equality rule, rejecting a
   conflicting key, domain, intent, candidate, or terminal claim as integrity failure;
6. otherwise performs an admitted mutation and records its canonical result atomically; and
7. only for an applied mutation, commits one permanent semantic-key result proof for future exact
   resolution.

`NonceDomainBusy`, `NonceLineageDiverged`, `NonceCapacityExhausted`,
`CandidateProgressionConflict`, `SupersededBeforeEntry`, and definite no-entry `SafeFailure`
responses mutate nothing and do not occupy or poison the reservation, candidate, or completion
operation key. They remain durably visible in that run's access observation. If a successful
mutation already owns the key, exact existing resolution still returns its permanent original
proof before considering any transient disposition.

For an ambiguous database acknowledgement while the affine invoker remains live, the authority
locks the current lineage and resolves the permanent key. A present key returns the original
applied result. Confirmed absence proves that this operation made no mutation because mutation and
key proof are atomic; the same invoker may re-execute the unchanged captured resource request and
use the new serialized disposition. These bounded database attempts are retry-transparent
internals of one affine invocation: no no-mutation disposition has semantic linearization or is
returned outside the authority until that invocation obtains one definite response. Thus a lost
unobserved `Busy` or conflict may be followed by a different final disposition, but two semantic
responses cannot escape one invocation. This is physical commit resolution, not a new Runtime
authorization, fresh provider observation, or semantic retry. Repeated acknowledgement ambiguity
repeats only resolve-or-proven-absent within the bounded invocation; inability to establish either
before its bound is `EntryUnknown`. A transient no-mutation response is therefore not cached
forever merely to make its lost acknowledgement reproducible.

Producer-bound precondition or witness references are not automatically part of permanent-key
equality. Reservation treats the pending-floor observation as creation-only evidence. Completion
compares the run-independent canonical terminal outcome and separately validates compatible
terminal witnesses. Candidate activation compares the canonical candidate descriptor, semantic
signer, and attested transaction hash; signer-attestation and replacement-evidence producer
references are validation evidence, not equality identity. Exact re-resolution always returns the
originally committed result proof; it does not rewrite that proof merely because a later caller
supplied fresh compatible evidence.

This ordering makes all rotation races explicit:

- mutation before rotation resolves to the original result;
- rotation before mutation proves that the protected mutation was not applied;
- lost acknowledgement resolves by permanent operation key; and
- a late old request whose target-bound capability was revoked is rejected before target entry,
  while an exact request already admitted on the current target may resolve `ExistingSame`.

`ExistingSame` is an internal resource-transaction resolution during the already authorized
invoker; it never mints another Runtime authorization.

`SupersededBeforeEntry` is not a state failure or `ResourceHistoryInvalid`. Runtime records the
protected-non-application outcome. The store leaves the semantic cursor at the resource state and
folds to `Refreshable(next_attempt_ordinal, public_lineage_head_ref)`. A separately assembled
current worker may authorize the same semantic request with its current physical binding and the
new attempt identity. The stale worker receives no current target-bound capability and cannot
retry itself.

### Pending nonce observation and allocation

After `ReadWalletNonceStatus` returns `Absent`, every attempt to create a new reservation consumes
one fresh:

```text
eth_getTransactionCount(sender, "pending")
```

observation from an injected EVM `Read` state. A same-intent status hit reuses the existing
reservation and does not allocate. The same pending RPC is used for first allocation and for every
later new-intent local-next check. Its registered invoker strictly decodes the bounded JSON-RPC
quantity; the `Read` settlement and an injected EVM `Pure` state validate the exact chain, sender,
route-generation, and observation provenance, then bind the result to the admitted pending-floor
policy. The RPC call and pure qualification happen before the resource transaction; no database
lock is held across network IO.

For domain `D`, EVM-derived semantic reservation key `K`, intent `I`, and qualified pending value
`Pq`, the resource transaction performs:

```text
verify qualified_domain_activation uses the exact current schema,
qualified registry lineage, permanent domain binding, D, store lineage,
chain, sender, issuer namespace,
replay-exclusion contract and exact exclusion disposition, canonical
finalized sender-nonce floor, exhaustive terminal prior-resource
disposition, and exhaustive target and sender-path fence disposition

exercise the fresh transaction-bound target-fence permit
require its target, database identity, store lineage, writer epoch, and
public incarnation equal the assembly-private qualified current store
incarnation

resolve existing (D, K) first:
    same activation record, submission_intent_id, intent digest,
    candidate family, and domain
        -> return original proof
    changed activation or intent under K
        -> integrity failure

lock and fence D
re-resolve (D, K) under the lock:
    same activation record and exact intent -> return original proof
    changed activation or intent            -> integrity failure

if D has no retained activation record:
    require a virgin retained lineage
    stage qualified_domain_activation for the successful reservation commit
otherwise:
    require its exact retained activation record

if another incomplete reservation exists for D:
    return typed NonceDomainBusy without mutation

if local_high_water is absent:
    require no reservation or completion record
    require Pq is within the protocol-valid nonce range
    require Pq
        == qualified_domain_activation
           .qualified_finalized_sender_nonce_floor
           .sender_nonce_at_finalized_block
        otherwise return typed NonceLineageDiverged without mutation
    candidate = Pq
otherwise:
    local_next = checked_add(local_high_water, 1)
    candidate =
        local_next   if Pq <= local_next
        no mutation  if Pq > local_next;
                     return typed NonceLineageDiverged

insert reservation(
    D,
    K,
    submission_intent_id,
    I,
    candidate_family_ref,
    candidate,
    qualified_pending_observation_ref,
    pending_floor_policy_ref,
)
bind staged qualified_domain_activation, if any
update local_high_water = candidate
commit
```

The qualified pending-floor reference is creation-only precondition evidence, not part of
reservation identity. `WalletNonceDomainActivationRecord` and its content reference are stable
domain identity. The composite registry-issuance proof and current-run producer references inside
`QualifiedWalletNonceDomainActivation` are validation/creation evidence, not identity. Every call
independently validates that evidence, then compares the exact
activation-record reference with the retained record. In the race where two runs both observed
`Absent`, the loser may present different valid registry, producer, and fresh pending-observation
references; exact existing-key resolution compares the permanent activation record, domain, key,
submission intent, intent digest, and candidate family and returns the original reservation
without replacing its retained creation evidence. A changed permanent identity is an integrity
conflict.

Two runs for different completed intents may observe the same `Pq`; the resource transaction
serializes their reservations to distinct monotonic nonces. Concurrent runs with the same exact
intent resolve the same reservation. A lagging provider cannot move local state backward.

An absent `local_high_water` is legal only for a virgin retained lineage with no reservation or
completion records. Any disagreement between the high-water mark and retained resource history is
an integrity fault.

The implementation rejects overflow as typed `NonceCapacityExhausted` and rejects policy or
retained-history mismatch as an integrity fault. A qualified pending value above `local_next`
always produces typed `NonceLineageDiverged` without mutation; the allowed provider-ahead jump is
exactly zero. It is an alert-worthy definite disposition, not a parked Runtime integrity fault,
so that run closes through its normal failure path and a later run may obtain fresh evidence.
Under exclusive sender ownership, silent catch-up would conceal an out-of-band sender, wrong
chain/route qualification, rollback, or provider disagreement. The provider is evidence for the
pending floor, not allocation authority.

Every sender-capable signer, relayer, operator path, stale deployment, and direct-submit path must
use this authority or be permanently fenced out. A pending provider read cannot close a race with
an uncoordinated actor.

### Stable intent and candidate family

The caller supplies a bounded idempotency token in an authenticated issuer namespace; it does not
supply the durable semantic ID directly:

```text
AuthenticatedIntentIssuerId =
    domain_separated_hash(
        "mfm.evm.intent-issuer.v1",
        admitted_tenant_id,
        stable_authenticated_client_principal_id,
        issuer_namespace_contract_ref,
    )

SubmissionIntentId =
    domain_separated_hash(
        "mfm.evm.submission-intent.v1",
        WalletNonceDomain,
        AuthenticatedIntentIssuerId,
        bounded_caller_submission_token,
    )
```

The stable authenticated client principal is mandatory, including a registered service principal
for system-originated calls. `issuer_namespace_contract_ref` is the admitted application-level
idempotency namespace and may span entry points only when qualification declares that sharing.
Neither value is a session, credential generation, or route. This identity is globally
collision-safe within the wallet-nonce domain even when tenants share a sender. The authenticated
issuer prevents one tenant or client from claiming another's ordinary caller token, but neither
issuer nor tenant partitions nonce allocation.
The namespace reference is immutable for the retained authority epoch. Changing it is the explicit
new-idempotency-epoch cutover and requires a new sender/domain; it is never an ordinary
implementation-contract or physical-generation upgrade.

The admitted request freezes one canonical `TransactionIntent` and one non-empty bounded
`CandidateFamily`. Their canonical content digest covers:

- the qualified chain instance and sender;
- destination, value, calldata, access list, transaction type, gas limit, and every other
  pre-reservation mutation or encoding field;
- the declaration-ordered candidate fee schedule and replacement bounds;
- the stable `SemanticSignerId` and exact deterministic signing-profile contract; and
- the exact semantic submission/expansion contract that interprets those fields.

It excludes `run_id`, occurrence and attempt identity, tenant routing, provider route, physical
signer generation, nonce-store writer generation, and other replaceable physical bindings.
Changing any semantic field changes the intent digest. Reusing one `SubmissionIntentId` with a
different digest is a permanent integrity conflict.

`TransactionIntent` and `CandidateFamily` are nonce-free templates: the authority-assigned nonce
cannot be part of their pre-reservation digest. `ReservedWalletNonce` then binds that nonce, and
every built/attested candidate includes it in the unsigned-candidate digest and transaction hash.
The intent also freezes the candidate derivation/encoding contract and terminal-assurance policy.
Each family member has one planning-fixed ordinal and differs only in the explicitly admitted fee
and replacement fields. Every member uses the same allocated nonce and is certified
mutation-equivalent: whichever member the chain accepts produces the same requested call
semantics. Candidate descriptors are canonical and pairwise distinct; after binding the nonce and
semantic signer, their unsigned digests and attested transaction hashes must also be pairwise
distinct. Certification or activation rejects a duplicate, so one winning transaction hash names
exactly one ordinal. The family bound makes every candidate, transaction hash, read, and branch
statically representable in the expanded program.

### Durable candidate progression

Reservation does not implicitly select a candidate. Candidate activation is a separate serialized
resource step:

```text
semantic_candidate_operation_key =
    domain_separated_hash(
        "mfm.evm.nonce-candidate.v1",
        semantic_reservation_key,
        candidate_ordinal,
)
```

The activation request has one closed permit:

```text
CandidateActivationPermit =
    Initial {
        current_reservation:
            ProducerBound<QualifiedCurrentWalletReservation>,
        exact_empty_activated_prefix,
        exact_next_ordinal: 0,
    }
  | Replacement {
        nonce_domain,
        semantic_reservation_key,
        candidate_family_ref,
        current_reservation:
            ProducerBound<QualifiedCurrentWalletReservation>,
        predecessor_activation_ref,
        predecessor_ordinal,
        exact_next_ordinal: predecessor_ordinal + 1,
        replacement_policy_ref,
        eligibility:
            ProducerBound<CandidateReplacementEligibility>,
    }

CandidateReplacementEligibility {
    nonce_domain,
    semantic_reservation_key,
    candidate_family_ref,
    predecessor_activation_ref,
    exact_observed_activated_prefix,
    decision_chain_head,
    declaration_ordered_no_terminal_observation_refs,
    replacement_policy_ref,
}
```

The eligibility value is produced by an exact registered EVM `Pure + Never` state from the
current status snapshot and committed bounded transaction/receipt/head reads. It proves only that
the frozen policy permits the statically next candidate at that observation point; it does not
claim the intent can never complete. An unstructured optional value, caller boolean, wall clock,
or adapter decision cannot authorize replacement.

Before activation, an injected signer `Read` named `AttestCandidateIdentity` receives the exact
unsigned candidate and produces only a secret-free attestation containing its unsigned digest and
deterministically derived transaction hash. It retains no signature or signed transaction.
Qualification proves that later `BroadcastExactCandidate` invocations under any admitted physical
generation of the same semantic signer reproduce that hash byte-for-byte.

Under the domain lock, `activate_candidate` enforces:

- exact existing-key resolution returns the original activation only when the reservation,
  ordinal, candidate bytes, semantic signer, and attested transaction hash agree;
- the first activation consumes `Initial`, has ordinal zero, expects no current candidate, and
  belongs to the frozen family;
- a later activation is exactly `j + 1`, names current ordinal `j`, belongs to the same family,
  and consumes `Replacement` with exact producer-bound committed EVM evidence accepted by the
  frozen replacement policy;
- the new canonical descriptor, unsigned digest, and attested transaction hash are each compared
  under the lock with every retained prefix member; any duplicate or cross-ordinal identity is a
  no-mutation integrity conflict;
- after exact successful candidate-key resolution has been checked first, a stale expected
  ordinal, competing progression, or already completed reservation returns typed
  `CandidateProgressionConflict` without mutation so the run must read status again; and
- every activation is retained permanently in ordinal order, while `current_candidate` names the
  latest member.

The authority validates the requested progression; it never chooses a fee, candidate, or
replacement. An older already-authorized broadcast may race a newer activation, so correctness
does not depend on only the latest member reaching the chain. All retained members are
mutation-equivalent and their known hashes remain terminal candidates. A later run reads the
complete bounded activated prefix, observes every relevant hash, and reproduces only the current
candidate when another submission is needed.

### Typed provenance without EVM-aware Runtime

The EVM domain owns privately constructible types conceptually equivalent to:

```text
ObservedPendingNonceFloor {
    nonce_domain,
    route_generation_ref,
    pending_nonce,
    observation_ref,
}

QualifiedPendingNonceFloor {
    observed: ProducerBound<ObservedPendingNonceFloor>,
    pending_floor_policy_ref,
}

EvmNonceReservationKey {
    semantic_reservation_key,
}

EvmCandidateOperationKey {
    semantic_candidate_operation_key,
    candidate_ordinal,
}

EvmNonceCompletionKey {
    semantic_completion_key,
}

ReadEvmWalletNonceStatusRequest {
    nonce_domain,
    qualified_domain_activation:
        ProducerBound<QualifiedWalletNonceDomainActivation>,
    semantic_reservation_key,
    submission_intent_id,
    transaction_intent_digest,
    candidate_family_ref,
}

ReserveEvmNonceRequest {
    nonce_domain,
    qualified_domain_activation:
        ProducerBound<QualifiedWalletNonceDomainActivation>,
    submission_intent_id,
    transaction_intent_digest,
    transaction_intent_ref,
    transaction_intent_object_closure,
    candidate_family_ref,
    candidate_family_object_closure,
    reservation_key: ProducerBound<EvmNonceReservationKey>,
    qualified_floor: ProducerBound<QualifiedPendingNonceFloor>,
}

ReservedWalletNonce {
    nonce_domain,
    domain_activation_record_ref,
    nonce,
    semantic_reservation_key,
    submission_intent_id,
    transaction_intent_digest,
    transaction_intent_ref,
    candidate_family_ref,
    observed_floor_ref,
    resource_lineage_ref,
    reservation_evidence_ref,
}

AttestedWalletCandidate {
    semantic_reservation_key,
    candidate_ordinal,
    candidate_descriptor_ref,
    unsigned_candidate_digest,
    transaction_hash,
    semantic_signer_id,
    signing_profile_contract_ref,
    signer_attestation_ref,
}

ActiveWalletCandidate {
    attested_candidate,
    activation_evidence_ref,
}

QualifiedCurrentWalletReservation {
    status_observation_ref,
    reservation,
    transaction_intent_object_closure,
    candidate_family_object_closure,
    activated_candidates,
    current_candidate: Option<ActiveWalletCandidate>,
    resource_head_ref,
}

ActivateEvmCandidateRequest {
    nonce_domain,
    candidate_operation_key: ProducerBound<EvmCandidateOperationKey>,
    next_candidate: ProducerBound<AttestedWalletCandidate>,
    activation_permit: ProducerBound<CandidateActivationPermit>,
}

WalletNonceStatus =
    Absent
  | Busy
  | Reserved {
        reservation,
        transaction_intent_object_closure,
        candidate_family_object_closure,
        activated_candidates,
        current_candidate: Option<ActiveWalletCandidate>,
        resource_head_ref,
    }
  | Completed {
        reservation,
        transaction_intent_object_closure,
        candidate_family_object_closure,
        activated_candidates,
        canonical_terminal_outcome_object_closure,
        completion_evidence_ref,
        resource_head_ref,
    }

CanonicalTerminalOutcome {
    nonce_domain,
    semantic_reservation_key,
    submission_intent_id,
    transaction_intent_digest,
    nonce,
    winning_candidate_ordinal,
    winning_activation_evidence_ref,
    transaction_hash,
    inclusion_block_identity,
    terminal_assurance_contract_ref,
    execution_disposition,
    canonical_public_result_object_closure,
}

CompleteEvmNonceRequest {
    nonce_domain,
    completion_key: ProducerBound<EvmNonceCompletionKey>,
    current_reservation:
        ProducerBound<QualifiedCurrentWalletReservation>,
    canonical_terminal_outcome: ProducerBound<CanonicalTerminalOutcome>,
    terminal_witnesses: ProducerBound<TerminalWitnesses>,
}

CompletedWalletNonce {
    nonce_domain,
    nonce,
    semantic_reservation_key,
    semantic_completion_key,
    canonical_terminal_outcome_object_closure,
    original_terminal_witnesses_ref,
    completion_evidence_ref,
}

CompletedProjection<Output, EvmSubmissionFailure> =
    Success(Output)
  | Failure(EvmSubmissionFailure)
```

`WalletNonceStatus` is one closed returned value, not generic query authority. `Busy` reveals no
other intent or reservation content. `Reserved` requires a contiguous bounded activated prefix and
`current_candidate == last(activated_candidates)` or both absent. `Completed` seals that prefix
and cannot coexist with an active marker. Its current Runtime-authorized `Read` observation is the
producer for the whole returned object closure. Therefore a later run can validate and project the
canonical completed result without dereferencing an arbitrary prior-run producer or teaching
Runtime how EVM evidence works.

Every `ReadEvmWalletNonceStatusRequest` carries only the stable permanent activation/domain-binding
evidence produced by `QualifyEvmNonceDomain`. Current-incarnation and target-fence evidence remain
assembly-private physical binding, so `SupersededBeforeEntry` refresh never changes canonical
semantic request bytes.

The reserve request carries the canonical `TransactionIntent` and `CandidateFamily` objects and
their complete bounded closures, not merely references requiring a cross-authority lookup. The
nonce authority verifies canonical bytes, digests, and reference equality and stores that closure
in its own content-addressed resource transaction with the reservation. It receives no generic
RunHistory object resolver.

`BindCurrentWalletReservation`, an injected EVM `Pure + Never` state, is the only path from the
`Reserved` variant to a resource mutation request. It consumes the exact current status
observation and emits `QualifiedCurrentWalletReservation`, preserving the embedded resource proof,
prefix, family, and status-observation reference under one current-run state-output producer.
`ProjectCompletedWalletDisposition`, also `Pure + Never`, consumes the exact `Completed` status
or completion response and emits one ordinary closed
`CompletedProjection<Output, EvmSubmissionFailure>`. A following certified structural `Match`
routes its success payload to the fragment normal channel or its failure payload to the fragment
failure channel. The pure state itself never constructs `StateOutcome::Failure` and the closed sum
is not an outcome instruction. A caller cannot lift an arbitrary embedded prior-run reservation or
completion reference directly into either producer-bound input.

Injected EVM `Pure` states derive stable operation keys:

```text
semantic_reservation_key =
    domain_separated_hash(
        "mfm.evm.nonce-reservation.v1",
        nonce_domain,
        submission_intent_id,
    )

semantic_candidate_operation_key =
    domain_separated_hash(
        "mfm.evm.nonce-candidate.v1",
        semantic_reservation_key,
        candidate_ordinal,
    )

semantic_completion_key =
    domain_separated_hash(
        "mfm.evm.nonce-completion.v1",
        semantic_reservation_key,
    )
```

The formulas contain no implementation or derivation-contract reference, so an implementation
upgrade cannot fork an in-flight semantic identity. The canonical request stores and validates
the exact semantic intent/expansion and derivation contracts instead. All three keys are stable
across physical attempt ordinals, exact re-resolution, binding refresh, and runs for the same
submission intent. Runtime and the run-history store carry the values and producer references as
opaque typed material and never derive them or inspect their EVM fields.

EVM states validate field-level agreement. The wallet-nonce adapter validates its resource
request, candidate progression, terminal claim, and durable proof. Runtime and the run-history
store validate only generic program, contract, occurrence, and producer-bound provenance.

Rust nominal types alone cannot prove chain/sender equality after deserialization. Private
constructors, exact certified producers, canonical bytes, store validation, and adapter-side
domain checks form the complete boundary.

### Completion and incomplete reservations

`CanonicalTerminalOutcome` is run-independent canonical content. It identifies one retained
activation, its transaction hash, the transaction's canonical inclusion block, success or revert,
the exact terminal-assurance policy, and the full canonical public projection value with its
canonical bytes and complete content-addressed object closure. A commitment without the object is
not a completable outcome. Later finalized-head observations are not part of canonical equality:
they belong to
`TerminalWitnesses` with the exact producer-bound receipt, head, inclusion, and finality evidence
that proves the claim. The outcome contains no `run_id`, occurrence, observation, route, or
physical-generation reference; its winning activation reference is the authority's permanent
secret-free resource proof, not a run producer.

Completion first resolves and re-resolves the stable semantic completion key under the same
serialized resource transaction rules as reservation. For an existing key:

- byte-equal canonical terminal-outcome content causes the authority to validate the new
  witnesses against that outcome and return the original `CompletedWalletNonce`, even when the
  producer and observation references differ; and
- a different canonical terminal outcome is an integrity conflict.

For the first completion, the transaction holds the domain lock and must:

1. require that the supplied reservation is still the domain's one active reservation;
2. require the winning ordinal, activation proof, and transaction hash to equal any member of the
   retained activated prefix, not necessarily its current last member;
3. validate the canonical outcome and witnesses against the stored intent and terminal-assurance
   policy;
4. seal the complete activated prefix;
5. persist the canonical terminal-outcome object closure, original witnesses, and permanent
   completion proof; and
6. clear the active marker while retaining the high-water mark.

Those changes commit atomically. Completion does not release or recycle the nonce. A
`WalletNonceStatus::Completed` read returns that object closure, so a racing or later run projects
the already completed result rather than attempting a conflicting completion.

The earlier run's failed or parked history status never blocks admission or execution of a new
run. A later run with the same `SubmissionIntentId` and exact intent digest resolves and reuses
the permanent reservation, activated prefix, and completion if present. A different intent
receives typed `NonceDomainBusy` while any reservation is incomplete. The authority never
allocates above incomplete work, so an ordinary failed run cannot silently create an EVM nonce
gap.

No timeout releases, transfers, or skips a reservation. This initial RFC completes only a
qualified terminal inclusion or revert of one activated family member and defines no implicit
transfer, cancellation, or gap-fill operation.

An incomplete reservation permanently binds its exact intent, candidate, signing, decoding,
replacement, and terminal-assurance contracts. A compatible software upgrade may resume it only
when the new qualification registry still implements those exact contract references. Before an
incompatible contract or schema cutover on the same sender, deployment activation must stop new
admissions and use the old qualified release to drive every `Reserved` status to `Completed`.
The new release refuses activation while any incompatible reservation is incomplete. If one
cannot be completed, the deployment keeps the old qualified path isolated for that sender or
moves the new contract to a new sender/domain; it does not reinterpret the record or accumulate a
compatibility execution path in the new Runtime.

Compatible implementation and physical-writer upgrades on the same sender retain the one current
schema and complete readable lineage, including every permanent identity and operation-key result.
An incompatible persisted-schema or issuer-namespace cutover always uses a new sender/domain.
Resetting only high water on the same sender is forbidden because it would permit an old semantic
intent to allocate again.

## EVM Without An Executor

### Registered structured expansion

An authored EVM submission call selects an exact semantic expansion contract. The expander
substitutes a structured fragment at that declaration slot, for example:

```text
DeriveTransactionIntent        [Pure]
QualifyEvmNonceDomain          [Pure]
DeriveSubmissionIntentId       [Pure]
DeriveEvmNonceReservationKey   [Pure]
ReadWalletNonceStatus          [Read]
Match status
  Completed ->
    ProjectCompletedWalletDisposition [Pure]
    Match completed disposition
  Busy -> typed submission failure
  Absent ->
    ObservePendingNonce          [Read]
    QualifyPendingNonceFloor     [Pure]
    ReserveWalletNonce           [Effect]
    ReadWalletNonceStatus        [Read]
    Match post-reserve status
      Completed ->
        ProjectCompletedWalletDisposition [Pure]
        Match completed disposition
      Reserved -> continue
      Busy -> typed submission failure
      Absent -> exact prior no-mutation response path
  Reserved -> continue
BindCurrentWalletReservation  [Pure]
Select current/next candidate
BuildUnsignedCandidate         [Pure]
AttestCandidateIdentity        [Read]
DeriveCandidateActivationPermit [Pure]
DeriveEvmCandidateOperationKey [Pure]
ActivateWalletCandidate        [Effect]
Match activation response
  Activated ->
    BroadcastExactCandidate        [Effect]
    ObserveActivatedTransactions   [bounded Read states]
    ObserveReceiptsAndFinality     [bounded Read states]
    VerifyCanonicalInclusion       [Pure]
    ReadWalletNonceStatus          [Read, at every reconciliation point]
    DeriveEvmNonceCompletionKey    [Pure]
    CompleteWalletNonce            [Effect]
    Match completion response
      Completed ->
        ProjectCompletedWalletDisposition [Pure]
        Match completed disposition
  CandidateProgressionConflict ->
    ReadWalletNonceStatus          [Read]
    Match reconciled status
```

This is a shape, not an implicit loop. The exact expansion statically contains one exhaustive
branch per bounded candidate-family ordinal and distinct state occurrences for every status read,
candidate observation, replacement decision, and completion attempt. Every changed nonce, fee,
route, candidate, or semantic request is therefore an explicit certified value or occurrence.

At entry, status controls the path:

- `Completed` projects the returned canonical outcome object closure through the pure closed
  disposition and structural `Match` immediately;
- `Busy` produces only the payload-free typed `NonceDomainBusy` submission failure;
- `Reserved` rebuilds the exact current activated candidate and observes the complete activated
  prefix before any new submission or replacement; and
- `Absent` alone permits a fresh pending-floor observation and reservation.

The `Absent` branch always re-reads status after `ReserveWalletNonce`: a same-intent run may have
reserved, activated, or completed between the first status observation and the mutation. Existing
reservation-key resolution may return the original reservation proof, while that required read
returns the authoritative current aggregate. `Completed` and the exact same `Reserved` intent
follow their normal paths; `Busy` exposes no foreign content and maps to its typed failure. A
post-reserve `Absent` is legal only when the reserve response was a no-mutation disposition and
follows that exact bounded failure/recovery branch. `Reserved(response) -> Absent`, a different
same-key intent, or a torn prefix is integrity failure. If a reservation has no activated member,
the fragment builds, attests, and activates ordinal zero. A replacement branch may activate only
the statically next member after its exact committed eligibility evidence. A status read after
`CandidateProgressionConflict` follows the winner's durable prefix or completed outcome.

The expansion fragment owns failure handling for its injected states and preserves the authored
call's external output/failure boundary. Runtime and store see only ordinary certified states and
structured control.

The authored submission boundary has one real closed `EvmSubmissionFailure` contract. Expansion
owns a finite exact mapper table from every injected leaf failure into that boundary. It must
include definite redaction-safe dispositions for unavailable transport/provider/signer,
destination rejection, exhausted bounded observation/replacement policy, `NonceDomainBusy`,
`NonceLineageDiverged`, and `NonceCapacityExhausted`. Malformed evidence, contract mismatch,
possible entry, and resource-history corruption remain blocking or parked under their generic
classification and cannot be mapped into that sum. The implementation plan freezes the exact
payload-free variants and each leaf mapper before schema generation; this is a closed domain
enumeration task, not an open ownership decision.

Expansion is selected by the exact semantic EVM operation and capability requirement, not merely
because a live implementation happens to use an EVM JSON-RPC transport.

### Signing and submission

`BroadcastExactCandidate` is one authorized bounded effect. The qualified signing profile must
certify deterministic RFC 6979 low-`s` signing: the same intent, nonce, candidate ordinal, unsigned
envelope, `SemanticSignerId`, and signing-profile contract reproduce byte-identical bearer bytes
and the same transaction hash across processes. Every physical signer generation admitted for
that semantic signer proves the same public key/address and deterministic behavior.

`AttestCandidateIdentity` invokes the qualified signer without submitting, verifies the semantic
signer binding, discards the generated bearer bytes, and returns only the expected unsigned
digest and transaction hash. `BroadcastExactCandidate` later passes the exact activated unsigned
candidate to a qualified signer once, requires the derived hash to equal the retained attestation,
submits those exact bearer bytes once, discards them, and returns only secret-free proof such as:

```text
SubmittedCandidateProof {
    candidate_ordinal,
    unsigned_candidate_digest,
    transaction_hash,
    semantic_signer_id,
    signer_generation_ref,
    signing_contract_ref,
    submission_contract_ref,
}
```

`AttestCandidateIdentity` is admitted as `Read` only when qualification proves the signer
operation is deterministic and has no externally meaningful semantic mutation; an internal HSM
audit counter may be operational but cannot affect the returned identity. Consuming quota,
approval tokens, anti-replay state, billing credit, rate-limit capacity, or any other externally
meaningful state is semantic mutation and makes the signer operation ineligible for `Read`. A
signer that cannot meet that contract is ineligible for this expansion—there is no fallback that
silently reclassifies attestation as an `Effect` with different crash semantics.

No signature or raw signed transaction enters retained state. There is no earlier
`PrepareSignedCandidate` effect that retains bearer bytes. Deterministic reproduction plus durable
candidate activation makes a later run's submission target-convergent: it observes every known
activated transaction hash and, if submission is still required, repeating the exact current
candidate's bearer bytes names the same EVM transaction. A signer that cannot qualify
byte-identical reproduction is ineligible for this path.

The adapter cannot choose another nonce, alter the candidate, replace fees, rotate semantic
routes, poll, rebroadcast in a loop, or decide terminal run meaning.

Process loss after broadcast authorization and before a committed observation leaves that run's
submission occurrence possible-entry ambiguous. The initial design does not reauthorize that
occurrence, and no later state in that run is reachable. A separately admitted run using the same
intent reads the durable activated prefix and may converge by observing or submitting the current
byte-identical candidate under the same nonce reservation; a different intent remains blocked by
`NonceDomainBusy`. A concurrently submitted older activated member is still an admitted
mutation-equivalent terminal candidate. No hidden adapter loop or run-history mutation is
permitted.

### Reads and terminal meaning

Transaction lookup, receipt lookup, and head observation are explicit bounded `Read` states over
the declaration-ordered activated prefix. `VerifyCanonicalInclusion` is a `Pure` state over their
exact committed values and can select only a retained activated hash; any additional network
observation is a preceding explicit `Read`. A bounded “not yet available” result is a reviewed
safe failure or closed output that feeds an explicit operation branch. Repeated polling is
expressed as a finite expansion of distinct read occurrences.

Every branch that would map a definite candidate, provider, destination, observation, or bounded
policy disposition to `EvmSubmissionFailure` first executes its own
`ReadWalletNonceStatus` reconciliation state:

- `Completed` projects the permanent canonical result;
- a changed current candidate or activated prefix follows the corresponding statically declared
  branch and observes that prefix;
- the unchanged incomplete status permits only that branch's exact reviewed failure mapping,
  current-candidate resubmission, or next-candidate activation; and
- a valid `ReadWalletNonceStatus` `SafeFailure` follows that Read state's exact settlement and
  typed failure plan, while `IntegrityFault` blocks; neither can masquerade as a status snapshot
  or bypass its certified continuation.

The failure decision linearizes at that authoritative status snapshot. A completion or activation
visible at or before the snapshot wins and must be followed. The read is not a lease and the
separate wallet and RunHistory authorities do not claim cross-store atomicity: another run may
complete after the snapshot but before this run commits its bounded failure. Such a failure means
only “this run did not obtain the terminal result under its finite policy,” never that the shared
intent cannot later complete. A subsequent run starts from the newer status and projects it.
Reconciliation does not execute after a possible-entry effect in the same run; that run remains
parked and a separately admitted run starts with status.

`CompleteWalletNonce` consumes the exact producer-bound completion key, qualified current
reservation, canonical terminal outcome, and terminal-witness references. It validates but does
not invent terminality.
The completion response or a later `WalletNonceStatus::Completed` object closure is the only input
accepted by `ProjectCompletedWalletDisposition`; its closed output is then structurally matched
into the fragment's normal/failure channels. No branch may close as success or revert after
terminal transaction evidence but before exact resource completion.

## Store And PostgreSQL Boundary

### RunHistory is an adapter and authority

Runtime depends on a narrow run-history port. Its implementation may use PostgreSQL, but it owns:

- append-only record and object persistence;
- per-run exact-head compare-and-append;
- logical-key idempotency;
- acknowledgement-ambiguity resolution;
- atomic object, fact, binding, transition, and closure visibility;
- writer generation and non-rollback lineage;
- callback-free structural and semantic fold validation; and
- purpose-specific read views.

These responsibilities do not belong to SQL, `sqlx`, or Runtime.

Production assembly exposes separately scoped capabilities:

```text
qualified infrastructure
  -> deployment-only authoritative fence issuer
       -> opens target-bound live sessions for an exact physical target/lineage/epoch
  -> wallet-activation registry administrative role/scope
       -> ActivationRegistryIssuerPromoter // deployment maintenance only
  -> wallet-activation registry public role/scope
       -> ActivationRegistryProofReader    // deployment/assembly only
  -> run-history database role/scope
       -> RunHistoryWriter   // consumed only by Runtime
       -> purpose-limited readers
       -> immutable physical-binding certificate verifier
  -> wallet-nonce database role/scope
       -> WalletNonceAdapter(
            sealed target-bound live session,
            OfflineActivationRegistryVerifier,
            immutable public proof closure,
          )                 // reachable only by its registered Runtime invoker
```

`mfm-storage-evm-postgres` owns both the append-only activation-registry schema/admin CAS plus its
deployment-only public proof reader/offline verifier and the distinct per-lineage nonce
schema/adapter. They use separate roles and pools. Runtime and normal request execution receive no
registry pool or role, registry administrative credential, or external fence issuer; the
registered nonce invoker receives only the already sealed adapter, immutable proof closure, and
offline verifier.

The certificate verifiers can prove only public binding membership and lineage from the admitted
immutable proof closure; they cannot query, sign, invoke, promote, or mutate a resource. The
run-history role cannot mutate nonce authority, the nonce role cannot mutate the activation
registry or run history, and the registry public role cannot mutate either. No generic owner,
application pool, or query capability survives assembly.

The two authorities may share one physical PostgreSQL deployment only when qualification proves
non-rollback lineage, stale/sibling-writer exclusion, backup, restore, and promotion for both.
Physical co-location does not merge schemas, roles, fences, or semantic algebras.

### Shared physical primitive, separate semantic authorities

Run history and nonce reservations may share lower-level immutable-object, transaction-helper, or
compare-and-append implementation code when that reduces code. They never share one semantic
transaction boundary, connection/role capability, lock, or callback spanning both authorities;
no cross-store atomicity is implied.

They retain separate semantic validators:

- the run-history store validates the structured five-record run algebra; and
- the wallet-nonce authority validates its resource algebra.

A universal tagged history engine with callbacks, planners, policy hooks, or workflow scheduling
would recreate the executor and is rejected.

### Resource mutation remains causally visible

The nonce transaction is not a `StateTransitionCommitted` record, but the interaction remains
causally visible:

```text
ExternalAccessAuthorized(Effect, ReserveWalletNonce)
  -> atomic resource operation
  -> ExternalAccessObserved
       Returned(reservation) | SafeFailure
         -> StateTransitionCommitted(consumes exact state-consumable observation)
       SupersededBeforeEntry(public_lineage_head_ref)
         -> Refreshable(next_attempt_ordinal)  // no semantic transition
       EntryUnknown | IntegrityFault
         -> parked or blocked                 // no semantic transition
```

Only the resource authority decides cross-run uniqueness. Only Runtime records why one run invoked
it and what completion that run observed.

## Failure And Crash Semantics

| Boundary | Durable run fact | Required behavior |
| --- | --- | --- |
| Expansion or preparation fails | Prior verified history only | No authorization and no live entry. |
| Authorization append is rejected or stale | No new authorization | Reload the verified cursor; do not invoke. |
| Authorization append acknowledgement is ambiguous | Authorization may exist | Resolve the original append identity; mint no authority from ambiguity. |
| Authorization positively commits | `ExternalAccessAuthorized` | Mint one affine authority for the exact operation. |
| Process dies before or during read invocation | Unmatched authorization | Preserve the exact waiting cursor and report `ReadCompletionUnknown`; the selected initial policy has no same-occurrence successor. |
| Process dies before or during effect invocation | Unmatched `Authorized<Effect>` | Keep that fold state and report possible-entry ambiguity; do not authorize another effect attempt. |
| Registered invoker returns | Authorization plus pending completion in memory | Totalize immediately and commit one linked observation before normal return. |
| Observation loses an exact-head race | Authorization plus stable pending material | Rebase append without reinvoking. |
| Observation acknowledgement is ambiguous | Observation may exist | Resolve the unchanged original append before rebase. |
| Journal is unavailable after invoker return | Pending material remains in live task | No normal successful drive result. |
| Process dies after invoker return but before observation commit | Unmatched authorization; pending material lost | Keep `Authorized<K>`; report `ReadCompletionUnknown` for a read or possible-entry ambiguity for an effect. |
| `Returned` or `SafeFailure` observation commits | Linked `ExternalAccessObserved` | Reload and run the pure settlement callback. |
| State commits typed failure | Failed transition and producer-bound failure | Enter only its exact structural failure continuation; do not erase the failure. |
| Default handler commits | Handler transition and scope failure | In an operation scope, atomically close when the root result is `OperationOutcome::Failure`; in a lane, produce the typed lane failure. |
| Custom handler selects recovery | Handler transition and closed route | Execute only the declared recovery branch. |
| Definite ordinary error closes run | `RunClosed { outcome_ref }`, whose exact referenced object is `OperationOutcome::Failure(...)` | A new invocation may create and execute another run independently. |
| Access `IntegrityFault` observation commits | `ExternalAccessObserved::IntegrityFault` | Fold to durable `BlockedIntegrity`; never construct domain failure. |
| Callback, codec, contract, invalid-evidence, or candidate fault is detected without a committed fault observation | Prior verified history only | Reject or report repeatably with component attribution; do not append a diagnostic event or alter the cursor. |
| Protected resource `Effect` binding is proven stale before entry | `SupersededBeforeEntry(public_lineage_head_ref)` observation | Keep the same state current; fold to the next `Refreshable` ordinal. Only a current qualified worker may reauthorize. |
| Resource `Read` target session is definitely stale or revoked | Reviewed `SafeFailure` observation | Settle through that Read state's exact typed failure contract; never construct `Refreshable`. |
| Resource acknowledgement is ambiguous while task lives | Run authorization; resource operation may exist | Under the domain lock, return the permanent applied proof when its key exists; confirmed key absence proves no mutation and permits only the same affine invoker to re-execute the unchanged resource request. |
| Process dies after a resource result exists but before run observation | Unmatched run authorization plus durable resource proof | Keep that run parked. The nonce authority's separately admitted status read lets another run consume the durable reservation, candidate prefix, or completion; same-occurrence recovery would still require a separate contract. |
| Committed observation exists but process dies before settlement | Exact observation in history | Recompute settlement without live IO. |
| Fan-out lanes complete in different physical orders | Ordered lane histories | Join results in declaration order. |
| Current effect is possible-entry ambiguous | Open run at exact effect cursor | Keep it parked with no legal successor in this RFC; do not advance or duplicate. |
| Final transition makes the root `OperationOutcome` derivable | Terminal transition | Append `RunClosed` with that outcome atomically; accept no later record. |
| Initial normalization derives the root `OperationOutcome` | No state transition is needed | Atomically append `RunAdmitted` and `RunClosed` with that outcome. |

## Enforcement

The cutover must make these properties structural:

- ordered builder handles prevent forward references and invalid branch/lane escapes;
- only `StateBinding` receives an executable occurrence identity; `MatchBinding`,
  `FanOutBinding`, lexical outcomes, lane outcomes, and structural joins cannot be authorized,
  scheduled, or named by `StateTransitionCommitted`;
- callback-free normalization crosses `Match`, fan-out entry/join, fragment boundaries, and typed
  results without emitting a control transition, stopping only at an `AtState`, an eligible lane
  state, or the exact root outcome;
- exactly one root `OperationOutcome` is derivable on every completed path, and no lane or
  unrelated nested scope can forge the containing operation's outcome;
- sealed block policies prevent lane fragments, matches, failure posts, and recovery routes from
  widening `Pure | Read`, restoring `Effect`, or constructing fan-out beyond remaining depth;
- expansion receives an affine protected slot and cannot duplicate an effect;
- expansion dependencies, depth, occurrences, branches, and fan-out are bounded;
- final certification contains no unresolved calls, wrappers, or unhandled fallible boundaries;
- `RunHistoryWriter` is non-cloneable and consumed by Runtime assembly;
- live adapters cannot depend on or construct run-history mutation authority;
- raw PostgreSQL mutation capability remains private to qualified storage assembly;
- `Prepared<K>`, `Authorized<K>`, `PendingObservation<K>`, and
  `CommittedObservation<K>` have private fields and sealed constructors;
- only a positively new authorization append creates `Authorized<K>`;
- the fold alone derives attempt ordinals and only `Refreshable` permits a next attempt;
- private writer credentials never enter programs, requests, observations, or history;
- Runtime dispatch is the only path from authorization to a registered invoker;
- invokers have no outer normal error after accepting authority;
- after authorization, settlement consumes only an exact `CommittedObservation<K>`;
- at most one unresolved authorization exists for an occurrence, and an unresolved
  possible-entry effect can never be bypassed;
- certified failure plans consume only exact producer-bound typed state failures, admit only exact
  handler entry or affine fragment-boundary propagation, prove one eventual handler, and lower
  handlers to ordinary `Pure` state bindings;
- effects cannot occur transitively inside fan-out, and nesting cannot exceed certified depth two;
- branch selection is derived, not caller-authored;
- fan-out results are joined in declaration order;
- Runtime and store interpret no EVM, nonce, wrapper, handler-origin, or telemetry-specific
  semantics; they follow only ordinary certified structure and generic provenance;
- all nonce-reserving workflows use the same qualified resource authority;
- all sender-capable actors use that authority or are fenced out;
- the permanent activation-registry domain key and unique activation-record identity admit only
  exact replay, while its separate store-lineage key admits only exact replay or a qualified
  current-incarnation promotion; domain issuance atomically binds both tables under the same
  lineage serialization point used by promotion, and its administrative role never reaches
  Runtime or normal request execution;
- every nonce-authority status read carries stable permanent domain-binding evidence and
  atomically opens its snapshot through the live target-bound session, and every mutation
  additionally exercises a fresh transaction-bound external-fence permit;
- the external fence authority has one non-rollback lineage, irreversible revocation, and
  sibling-issuer exclusion; copied databases, credentials, public proofs, static tokens, and
  replayed permits are not authority;
- normal nonce reads and mutations perform no activation-registry IO or cross-authority lock;
- one authenticated issuer-scoped caller token derives one domain-global submission intent, whose
  stable reservation, candidate, and completion keys contain no `run_id` or physical generation;
- one incomplete reservation retains one bounded mutation-equivalent candidate family and one
  serialized activated prefix, so a later run observes every possible winning transaction hash;
- terminal completion compares run-independent canonical outcome content while independently
  validating producer-bound witnesses, and completed status returns the object closure required
  for pure projection;
- run-history and nonce database roles cannot cross-write;
- adapters contain no history fold, next-plan selector, semantic retry loop, or terminalizer;
  the wallet authority's bounded physical resolve-or-proven-absent commit resolution is the sole
  explicit exception and never mints Runtime authority; and
- replay, telemetry, and applications receive read-only purpose capabilities.

Architecture scans supplement but do not replace type privacy, crate dependency contracts,
compile-fail tests, hostile-history tests, backend conformance, and fault injection.

## Why Not Encode Every Audit Step As A State?

Authorization and observation surround external IO:

```text
authorization committed
  -> external operation
  -> observation committed
```

Renaming them semantic state transitions does not make them atomic or guarantee the second append.
It would put worker timing and crash mechanics into domain state.

Meaningful policy, validation, and domain work should be small injected states. Runtime mechanics
remain private typed protocol phases.

## Why Not Keep An Arbitrary DAG?

The current product needs:

- declaration-ordered sequencing;
- exhaustive conditional branches;
- bounded pure/read fan-out; and
- typed joins at structured boundaries.

An arbitrary DAG additionally requires cycle/reachability validation, alternative-source
materialization, global readiness, dependency skips, required-success sets, all-nodes-terminal
closure, conflict analysis, and fairness rules.

The repository-wide inventory found exactly three production `Operation` implementations plus
linear test fixtures. None requires overlapping joins, arbitrary producer alternatives, or
unstructured acyclic sharing. The portfolio operation compiles to a network fan-out containing
each child EVM operation's read fan-out, within the selected depth-two bound. External consumers
receive the declared breaking API. If a future use case needs more, it must first prove that the
structured algebra cannot express the required semantics; it must not reintroduce a graph merely
as an authoring convenience.

## Why Not Keep A Generic Executor?

A separate durable coordinator may be justified for a product requiring:

- arbitrary non-idempotent and non-queryable destinations;
- progress independent of a run being driven;
- cross-run coalescing of one logical delivery;
- independently operated delivery infrastructure; or
- open-ended scheduling.

Those are not current MFM requirements. Keeping a generic executor imposes a second interpreter,
history, fence, recovery algebra, and type system on every effect.

A future product needing those properties should integrate an explicit external authority or
propose a new boundary. It must not grow an adapter into another hidden Runtime.

## Why Not A Universal Outbox?

A universal outbox near every provider could preserve stronger forensics across process loss. It
would also require another durable writer or destination participant, fencing, ambiguous-commit
recovery, broader retention, secret review, and another availability dependency.

The baseline records authorization before entry and every normal completion afterward. Process
loss leaves honest possible-entry ambiguity. The initial design parks an ambiguous effect rather
than automatically re-entering it.

## Persisted Contract And Cutover Boundary

This target changes persisted semantics:

- the authored and certified graph become structured authored/expanded/certified programs;
- operation success and failure become the typed root block result, with no outcome instruction,
  occurrence identity, or separately writable control record;
- declaration order becomes semantic and hashed as order;
- node IDs become structured occurrence identities;
- arbitrary input bindings become lexical producer references and structured joins;
- `DependencySkipped`, blocking-source evidence, and its batch purpose disappear;
- required-success nodes and all-nodes-terminal closure disappear;
- node phase maps become a structured cursor and fan-out lane states;
- branch choice becomes derived from kernel-owned canonical closed-sum tags;
- state failure becomes a first-class producer-bound value for exact handlers;
- wrapper and failure-handler provenance enters the admitted expanded program;
- authored child calls lower completely into lexical structured blocks;
- physical access-attempt ordinals and refresh typestate become generic history state;
- executor `ensure` becomes direct `Effect` access;
- executor-retained closure and histories disappear;
- effect ambiguity no longer permits automatic overlapping re-entry;
- stale resource binding becomes protected-non-application evidence and a fold-derived
  `Refreshable` attempt;
- nonce reservation always consumes a fresh pending provider observation; and
- EVM submission identity, candidate activation, and canonical completion become permanent
  cross-run wallet-authority records; and
- EVM topology becomes registered structured expansion.

The implementation therefore requires one new sole current schema lineage, annex, corpus,
store/replay fold, trace/export projection, and public contract. Existing bytes are rejected; they
are never reinterpreted.

No dual reader, dual writer, graph lowering compatibility path, fallback executor, or parallel
direct-effect path is part of this RFC.

## Design Cutover Scope

The completed cutover must:

- replace authored/expanded graph public types and builders with the structured program algebra;
- make `State`, `Match`, and `FanOut` the only author-visible structural forms and lower root
  success/failure helpers to one typed `OperationOutcome`, never an instruction;
- replace arbitrary node bindings with lexical typed handles, branch merges, and fan-out joins;
- rewrite pure operation, child-operation, framework, and capability expansion as typed structured
  substitution;
- bind the exact trusted program and predicate contracts, required expansion profile, and coverage
  policy at entry-point admission;
- add exact expansion profile ordering, bounds, provenance, and affine protected slots;
- add lexical operation/lane default failure-handler state contracts and producer-bound failure
  values;
- rewrite certification for lexical dominance, exhaustive branches, handler coverage, fan-out
  restrictions, capability closure, and root-outcome totality;
- replace graph readiness and scheduling with cursor interpretation;
- replace graph-shaped store/replay folding with the structured callback-free fold;
- delete dependency skips, alternative-source selection, cycle/reachability proofs,
  required-success sets, authorization-count scheduling, and graph-wide phase maps;
- delete `crates/kernel/executor`;
- delete executor storage crates, schemas, manifests, ledgers, frontiers, target authorities,
  tombstones, fences, and tests;
- remove EVM wallet history folding and next-plan selection from the live layer;
- add direct typed effect access under Runtime's private bracket;
- add registered EVM structured expansions and the narrow wallet-nonce authority, including stable
  intent derivation, pending-floor allocation, serialized candidate activation, purpose-limited
  status, and canonical terminal completion;
- add the separately credentialed append-only wallet-activation registry admin CAS/read-only
  verifier and the deployment-owned target-bound physical fence, with promotion ordered after
  complete-prefix proof and irreversible old-target/sender-path fencing;
- keep raw PostgreSQL pools private and expose separately scoped qualified authorities;
- update `docs/design.md`, `docs/architecture.md`, run-execution documentation, persisted-surface
  inventories, qualification docs, app wiring, CLI/REST projections, and relevant READMEs;
- reset the sole current persisted contract deliberately; and
- delete all superseded dependencies, tasks, fixtures, migrations, tests, and terminology.

The readiness decisions below are frozen for the initial cutover. Environment-specific legacy
deployment inventory remains an activation gate, not an implementation-design dependency.

## Implementation Acceptance Verification

Implementation must validate the design with small models and boundary tests, not a parallel
production path:

- compile every current production operation into the structured algebra;
- prove no current operation requires arbitrary DAG-only behavior;
- model one success/failure branch, one recovery branch, and one nested child-operation failure;
- prove child success/failure outcomes lower to call-site success/failure without closing the
  parent;
- prove root success/failure helpers create no binding, occurrence identity, cursor position, or
  standalone history record;
- reject nominal substitution among `StateOutcome`, `LaneOutcome`, and `OperationOutcome` despite
  identical tag/payload shapes;
- reject occurrence IDs, access state, transitions, and authorizations attached to `Match`,
  `FanOut`, a fragment boundary, or any outcome; prove their callback-free normalization emits no
  control record and reaches only the next state occurrence, eligible lane state, or root outcome;
- prove exactly one root outcome is derivable on every completed path and reject a lane, child
  fragment, or unrelated lexical scope that attempts to forge the containing operation's outcome;
- reject closure with the wrong `OperationOutcome` variant, nominal contract, lexical value
  derivation, inactive branch value, missing or unbound outcome object, second inline outcome
  encoding, or delayed/standalone append; for admission-only fan-out, require the complete
  lane-wrapper/join object closure in the same append while retaining exactly one root outcome;
- exercise both root variants over each applicable `LexicalValueRef` provenance kind—admission
  root, state output, arm value, variant payload, fragment input, fragment boundary, and fan-out
  join—and reject cross-kind, wrong-source, wrong-selector, wrong-tag, wrong-payload-path,
  wrong-child-root, wrong-lane-order, and wrong-contract substitution; for arm, variant-payload,
  and fan-out-join derivations, also reject correct provenance paired with a wrong or unbound
  content reference and recompute the one canonical reference from the sealed constructor; for a
  variant payload, derive its type and nominal contract exclusively from the selector contract's
  exact tag/path table and reject byte-identical payloads registered under a distinct contract;
- prove child substitution binds every declared input root exactly once from its exact dominating
  call-site slot, symmetrically binds exact child normal/failure tail slots to the one fragment
  boundary, and rejects missing, duplicate, extra, inactive, wrong-role, or contract-substituted
  inputs/boundaries; prove certification contains no resolved fragment value reference;
- require every `Match` selector to be an exact already-defined closed-sum slot and reject a
  future reference, non-dominating slot, or structurally similar non-registered sum;
- reject omitted `RunClosed` when admission or a transition first derives the root outcome, and
  reject premature closure before it is derivable;
- prove byte-identical expansion under registry and map iteration variation;
- construct exactly one canonical `CertifiedProgramRef` over the complete expanded program,
  profile, proof, manifest, contract, bound, and implementation closure; reject an independently
  substituted component or audit projection even when every substituted object is otherwise
  individually valid; prove the closure preimage excludes its digest and root, is stable under
  object-store and map iteration variation, deduplicates a shared DAG object on first visit, and
  rejects a transitive reference cycle, missing object, wrong object-type tag, or unregistered
  outbound-reference position;
- resolve the exact entry-point admission policy from qualified registry identity, require its
  reference and every bound program/predicate/profile/policy field in the certified root, and
  reject a caller-selected, stale, unqualified, or weaker policy or a valid proof evaluated under
  a different predicate set;
- prove exact wrapper nesting, affine core use, expansion termination, and hard bounds;
- prove all injected fallible states and fragment boundaries receive exactly one handler;
- prove only exact `KERNEL_NEVER_FAILURE_CONTRACT_REF` constructs `Infallible`, reject separately
  registered look-alike uninhabited contracts and distinct-reference aliases, accept only the
  sealed kernel `Never` registration path (including a source type alias to that exact type),
  reject a handler on the reserved contract, and require a plan for every typed contract;
- prove `Never` constructs `NoFailure` with no failure slot, producer, or plan, while every
  `FailurePlan` begins at an exact typed certified slot; reject a certified program containing
  future content references or claimed committed transitions; reject state and fragment failure
  slots with the correct Rust type but a different retained failure-contract reference;
- for `Pure`, `Read`, and `Effect`, test both the reserved `Never` reference and other failure
  contracts; prove a `SafeFailure` cannot construct failure or handler entry for an infallible
  state;
- reject every proposed failure and every `StateOutcome::Failure` transition for an infallible
  `Pure`, `Read`, or `Effect` occurrence, prove a callback proposal has no committed nominal
  authority before exact-head append, and reject an infallible capability/state pairing whose
  admitted `SafeFailure` needs a negative disposition;
- reject a default handler whose execution kind is not `Pure` or whose exact failure contract is
  not `KERNEL_NEVER_FAILURE_CONTRACT_REF`; require its one `Propagate` failure tail to use the
  exact variant-payload slot derived from that handler output and reject a different dominating
  same-typed value, intervening binding, wrong payload path, wrong content reference, or
  byte-identical payload under a distinct nominal contract; require exact equality among the
  enclosing scope failure contract, registered `Propagate` payload contract, derived payload
  slot contract, and failure-tail contract;
- for an operation, lane, or fragment scope with the reserved `Never` failure contract, reject
  `.or_default()`, reject construction of a scope/root/lane failure result, and require every
  explicit handler route for an owned fallible source to recover totally;
- reject every non-local successful scope escape, any direct normal-control escape that uses the
  protected failure before its designated handler, a forged affine propagation target, and a
  propagation chain with zero or multiple eventual handlers; separately prove that a distinct
  newly committed pre/post/support-state failure follows its own exact plan and may causally
  supersede the protected failure without reusing it;
- reject an affine failure-mapping chain with the wrong source, plan identity, lexical region,
  mapped target, or fragment-boundary rebind; a non-`Pure`, foreign, or inactive-path mapper
  binding; broken adjacent slots/contracts; a future committed-reference claim; or a zero-link
  source/boundary contract mismatch;
- reject substitution of a same-typed failure slot from another source for
  `ExactResultSlotOf<before_handler>` and substitution of another same-typed route slot for
  `ExactOutputSlotOf<handler>`; prove the source binding plus structural plan path derives the one
  plan identity and reject any author-supplied, cross-plan, or same-contract substituted
  `FailurePlanBound`;
- reject forward references, inactive-arm escape, cross-lane values, incompatible merges,
  effects in fan-out, operation outcomes in lane blocks, and unbounded fan-out;
- reject an effect hidden anywhere inside fan-out and reject fan-out beyond certified depth two,
  including through a fragment, `Match` arm, failure-post path, or custom recovery route;
- prove collect-all fan-out under every physical completion-order permutation;
- prove lane success and failure each construct exactly one canonical `LaneOutcomeRef`, prove
  deterministic non-empty joins at both supported depths without a control transition or
  occurrence identity, and reject an empty group;
- reject an omitted, duplicate, foreign, misordered, or contract-substituted canonical lane
  wrapper, including when multiple lanes select byte-identical admission-root payloads;
- prove store rejection of skipped pre/post/handler occurrences and forged branch selection;
- prove store rejection of authorization for future state occurrences, wrong lanes, wrong bindings,
  wrong attempt ordinals, and any second unresolved same-occurrence access;
- reject observation variant, schema, and contract substitution; prove only the exact certified
  `Returned` or `SafeFailure` completion reaches settlement and that physical-control,
  possible-entry, and integrity evidence cannot be laundered into either variant;
- prove only a committed `ExternalAccessObserved::IntegrityFault` derives durable
  `BlockedIntegrity`; callback, codec, contract, settlement-invalid-evidence, and rejected-candidate
  faults append no diagnostic record, preserve the authoritative cursor, and repeat with exact
  component attribution;
- crash at every authorization, invocation, observation, settlement, handler, branch, fan-out, and
  closure boundary;
- for a fallible state under a non-reserved operation-scope default, prove a definite typed
  negative response and an exact `SafeFailure -> StateOutcome::Failure` mapping each reach that
  default handler, close the run, and do not block a new run;
- qualify fallible `Read` and `Effect` `SafeFailure` mappings separately and reject any effect
  `SafeFailure` whose evidence leaves target application unknown; that evidence must remain
  `EntryUnknown`;
- prove possible-entry effect ambiguity cannot mint another authorization;
- fault-inject resource rotation versus mutation and prove `SupersededBeforeEntry` or exact
  internal `ExistingSame`;
- prove only `SupersededBeforeEntry` creates `Refreshable`, exactly one next ordinal is possible,
  stale workers cannot self-upgrade, and target-bound sessions, transaction permits, fence keys,
  and private writer credentials never persist;
- prove an initially derivable root outcome atomically appends `RunAdmitted` and `RunClosed`;
- crash an unmatched read inside and outside fan-out and validate the selected read-recovery
  policy;
- model concurrent runs observing the same pending nonce and prove unique monotonic reservations;
- prove first-use and later reservation use the fresh pending-floor algorithm, validate the virgin
  value against the protocol nonce range, and treat a retry's different fresh observation as
  creation-only evidence when the permanent reservation identity already matches;
- prove reservation receives, validates, and atomically retains the canonical intent and candidate
  family object closures plus the exact
  `ProducerBound<QualifiedWalletNonceDomainActivation>` and its current-schema record closure in
  the nonce authority without generic RunHistory query authority; reject a foreign-run,
  wrong-producer, or same-shaped activation value;
- prove every no-mutation nonce response leaves its permanent semantic operation key unoccupied,
  while an already applied exact key resolves its original proof first;
- lose acknowledgement for `NonceDomainBusy`, `NonceLineageDiverged`, and
  `CandidateProgressionConflict`, change domain state, and prove key absence permits only the same
  live affine invocation to re-execute the unchanged request without a second Runtime
  authorization or fresh pending observation; prove those internal attempts have no semantic
  linearization until one final disposition escapes and can never expose two responses;
- prove independently operated forks sharing chain ID and genesis cannot qualify as one chain
  instance, while redundant routes to one qualified instance do; reject registry aliases,
  namespace reuse, wrong fork anchors, and route membership in two instances;
- prove the current tree, dependency graph, features, binaries, and maintenance tasks contain no
  retired-schema reader, decoder, migration, or certifier; retain an old export only as opaque
  offline audit material and reject every retired byte sequence at every current boundary;
- qualify the exact current-schema wallet-domain activation record and reject a wrong store
  lineage, writer epoch, issuer namespace, chain/domain/sender, replay-exclusion disposition,
  non-terminal prior allocation or submitted candidate, canonical finalized block or sender-nonce
  floor, incomplete sender-path inventory, or writer/signer/relayer/direct-submit fence;
- prove a reused sender activates only after every old effect is definitely terminal, every retry
  ingress and sender path is fenced, and a new idempotency epoch is qualified; otherwise require a
  new sender/domain; prove its first reservation binds that activation record and uses the
  ordinary fresh virgin pending-floor algorithm without importing high water; require pending to
  equal the qualified current-chain finalized sender-nonce floor and reject both lagging and
  provider-ahead values without mutation;
- race concurrent first reservations and reject a second or conflicting activation record; prove
  there is no bootstrap mutation, credential, role, result, or retry path; and
- race different store lineages for one unclaimed domain through the activation-registry CAS and
  admit exactly one permanent lineage/activation identity; prove exact replay returns the original
  composite proof, a different permanent binding always conflicts, one lineage may serve multiple
  domains, and a crash after issuance pins the domain before its first local reservation;
- race two new domains proposing sibling initial targets for one lineage, race domain issuance
  against lineage promotion, and crash between each internal registry statement; prove one
  lineage-key serialization point, all-or-nothing lineage-head plus domain-row visibility, no
  partial binding, and exact composite-proof resolution after acknowledgement ambiguity;
- copy the database, lineage, writer epoch, configuration, public incarnation proof, client
  credential, and static attestation to two physical targets; prove only the target holding the
  non-exportable key and live externally fenced session can return semantic status or reserve,
  activate, or complete;
- before a first local reservation, present a domain binding for another lineage or omit its
  permanent proof and prove `read_status` cannot return `Absent`; prove every authoritative status
  read atomically opens its snapshot through the live target session, every mutation uses a
  distinct transaction-bound non-replayable fence permit, stale capabilities fail for all four
  operations, and normal reads and mutations issue zero activation-registry calls;
- race old/new target operations and crash before old-target fencing, after fencing but before
  registry CAS, after CAS but before replacement opening, and after opening; prove unavailable-safe
  exact retry and complete-prefix preservation; commit between a premature snapshot and fencing
  and prove that snapshot cannot qualify because the accepted fence and prefix proofs must bind the
  same post-quiescence final old head; prove the old target plus every old
  writer/signer/relayer/direct-submit path and outstanding transaction can never regain
  reachability; and
- fault-inject activation-registry rollback/sibling writers and external-fence-authority
  rollback/sibling issuers, stale-session resurrection, and replayed transaction permits;
  qualification must fail closed, while any unprovable target fence, sender-path fence, or complete
  prefix forbids same-domain promotion and requires a new sender/domain;
- reject an issuer-namespace change or incompatible persisted schema as an ordinary same-sender
  upgrade; require a new sender/domain, while compatible upgrades preserve the complete readable
  current-schema lineage and permanent operation-key algebra;
- prove authenticated issuers using the same caller token derive distinct collision-safe
  submission IDs and cannot claim each other's intent;
- prove the EVM reservation, per-ordinal candidate, and completion keys are stable across exact
  re-resolution, implementation-contract upgrades, physical refresh, and different `run_id`
  values for the same `SubmissionIntentId`; prove that a changed intent digest or candidate family
  under that ID is an integrity conflict;
- race candidate activation across runs and prove one serialized contiguous prefix, exact
  `CandidateProgressionConflict`, no skipped ordinal, and no candidate outside the frozen
  mutation-equivalent family; reject under the domain lock any descriptor, unsigned digest, or
  transaction hash duplicated across ordinals;
- reject a replacement without the exact closed `CandidateActivationPermit`, predecessor
  activation, next ordinal, current status producer, policy, and committed eligibility evidence;
- prove a later run receives the full activated prefix, observes every possible winning hash,
  reproduces the current candidate, and can complete when an older activated replacement wins;
- prove qualified physical signer generations implement one stable semantic signer and reproduce
  each attested transaction hash; reject a changed public key, address, algorithm, or signing
  profile, and reject `Read` qualification when attestation consumes quota, approval, anti-replay,
  billing, rate-limit, or other semantic state;
- prove byte-equal canonical completion resolves one permanent result when terminal witnesses have
  different valid producer/provenance references, and reject a conflicting canonical outcome;
- reject completion or completed status whose canonical public-result object or any required
  object-closure member is absent even when its commitment is present;
- under the domain lock, reject completion for a non-active reservation or non-activated
  ordinal/hash and prove an older activated winner atomically seals the prefix, persists the
  inclusion-block outcome, clears the active marker, and retains high-water;
- race status against activation and completion; prove a changed prefix follows its exact static
  branch, completed status supplies a current producer-bound object closure sufficient for pure
  projection, any completion visible at the status linearization point wins, and a later
  completion remains available to the next run rather than changing the earlier run's bounded
  failure;
- prove status comes from one current-lineage transactionally consistent snapshot; reject a stale
  replica, torn prefix, non-last current candidate, completion with an active marker, or an
  unqualified embedded reservation; prove only `BindCurrentWalletReservation` can mint the
  current-run producer accepted by activation/completion;
- test lagging, equal, and provider-ahead pending observations under the selected qualified policy,
  including rejection without mutation;
- prove one incomplete intent blocks a different intent without allocating a higher nonce and that
  a same-intent later run resolves the original reservation, candidate prefix, or completion;
- prove incompatible deployment activation is rejected while an incomplete intent binds a
  retired semantic contract; permit only exact-contract-compatible resumption, complete under the
  old qualified release, or a new sender/domain; even after completion, require a new
  sender/domain for an incompatible persisted-schema or issuer-namespace change;
- reject nonce-domain, qualified-chain-instance, issuer, intent, candidate-family, candidate,
  semantic-signer, observation, reservation, canonical-outcome, terminal-witness, and producer
  substitution;
- prove non-rollback resource and activation-registry lineages plus stale/sibling-writer exclusion
  across restore, promotion, and generation rotation; reject a restore that omits, rewrites, or
  rebinds any immutable historical activation binding, incarnation, target-key record,
  domain-activation record, permanent operation result, or member of their complete closure; allow
  the current incarnation and target key to differ only as a newly appended qualified promotion
  successor that retains the entire immutable prefix;
- inventory every signer, relayer, operator, stale deployment, and direct-submit path;
- prove run-history, nonce, activation-registry admin, and activation-registry public roles cannot
  cross-write; prove the registry admin and fence-issuer credentials cannot escape deployment
  maintenance, and no generic pool survives assembly;
- prove best-effort telemetry failure cannot affect a run;
- prove purpose-limited prior-run fact reads are complete through the exact authorization-captured
  `TenantFactFrontier` and cannot obtain generic query or append authority;
- prove `AttestCandidateIdentity` retains only the expected hash, prove
  `BroadcastExactCandidate` signs once and submits the exact derived envelope once, prove neither
  retains bearer bytes, and reproduce byte-identical envelopes across processes for the same
  intent and candidate ordinal;
- use canary credentials/provider text to prove no secret-bearing value reaches programs,
  histories, objects, errors, traces, exports, or logs;
- inventory every current executor predicate and assign it to expansion, certification, Runtime,
  store, resource authority, explicit state, or deletion; and
- show that no required predicate remains ownerless.

The implementation plan assigns these checks to the commit that first owns each boundary and to
the final qualification hardening commit.

## Readiness Decisions

The repository audit, formal failure review, access/resource review, nonce review, and dedicated
architecture review freeze the following initial-cutover choices:

| Area | Frozen decision |
| --- | --- |
| Expected operational dispositions | Only reviewed `Returned` and redaction-safe `SafeFailure` evidence is state-consumable. Every valid expected negative disposition maps to a closed typed state failure. `SupersededBeforeEntry`, possible entry, malformed evidence, and integrity faults retain their distinct non-domain meanings. |
| Failure defaults | Each typed-failure operation, fragment, or lane owns a finite exact source-contract-to-`Pure + Never` mapper table. `.or_default()` requires exactly one match. A `Never` scope has no default and explicitly recovers every owned fallible source. |
| Expansion | Child substitution, capability lowering, policy wrapping, failure completion, then normalization/certification is the one acyclic pipeline. Support states are executable leaves and do not recursively expand; cross-policy support requires an explicit composite expansion. |
| Security wrapper denial | A denying wrapper supplies an exact mapper into the protected typed failure boundary. It cannot wrap `Never` or widen a boundary. Non-denying `Never` wrappers contain only `Never` support states. |
| Fan-out | Fan-out is planning-fixed, non-empty, collect-all, `Pure | Read`, homogeneous per group, and declaration ordered. It may nest to certified depth two under global lane/occurrence bounds, preserving the portfolio network fan-out and each child EVM read fan-out. |
| Unmatched Read | The exact occurrence remains `Authorized<Read>` and reports `ReadCompletionUnknown`. The initial contract has no same-occurrence reauthorization, timeout supersession, synthetic completion, or interruption. A separate run may execute. |
| Possible-entry Effect | The exact occurrence remains parked. Only qualified `SupersededBeforeEntry` creates `Refreshable`; no automatic re-entry or force-close exists. |
| Structured-program coverage | All three repository production operations and the linear fixtures fit the structured algebra. External graph-authoring consumers receive the declared breaking API; no compatibility lowering exists. |
| Certification authority | One canonical `CertifiedProgramRef` binds the exact authored/expanded programs, profile, proofs, contracts, bounds, manifests, and secret-free implementation closure. Its entry-point policy is resolved from qualified registry identity and trust-anchors the exact program contract and predicate set; the root cannot select weaker certification rules. Its closure digest excludes itself and the root and uses the registered tagged depth-first walk, active-stack cycle rejection, and first-visit DAG deduplication. Repeated admission fields are equality-checked audit projections, never independent authority. |
| Lexical result algebra | Non-local successful `ScopeResult`, `ArmResult`, and `RecoveryResult` are deleted. Every block has one normal value channel and one lexical failure channel; root/lane/fragment outcomes are nominal structural wrappers, never instructions. |
| Structural value references | Arm aliases reuse the exact source reference; variant payloads and fan-out joins derive one canonical content reference from their exact source objects. A variant payload's type and nominal contract come only from the exact selector contract's tag/path table, so byte identity cannot substitute another contract. Only sealed fold constructors can create these references. |
| Child inputs | Child substitution bijectively rebinds every declared child input root from an exact dominating call-site slot through a structural `FragmentInput` derivation. It creates no state, copied object, or new producer. |
| Failure contract authority | `RetainedValueContract` remains the sole descriptor of inhabited retained values; its canonical content reference is exact contract identity. Certified `LexicalSlot` and resolved `LexicalValueRef` separately own producer shape and identity. |
| Default failure propagation | `.or_default()` uses one `Pure + Never` mapper whose sole `Propagate` payload is exposed as an exact variant-payload slot and is itself the scope-failure tail. The enclosing scope, route entry, payload slot, and tail carry one exact nominal contract; same-typed, byte-identical-distinct-contract substitution and intervening bindings are illegal. |
| `Never` | `FailureContract::Never` is a kernel sentinel resolving only to `KERNEL_NEVER_FAILURE_CONTRACT_REF`. It is not a retained-value schema and has no `MfmValue`, codec, value slot, decoder, or producer. |
| Integrity blocking | Only committed `ExternalAccessObserved::IntegrityFault` folds to durable `BlockedIntegrity`. Callback, codec, contract, invalid-evidence, and rejected-candidate faults leave the prefix unchanged and are repeatable attributed diagnostics; no sixth run event exists. |
| Prior-run facts | Fact selection is an ordinary `Read`, while the existing authorization-captured `TenantFactFrontier`, affine scan permit, completeness attestation, and purpose-limited scanner remain authoritative behind its invoker. |
| Pending nonce | Every reservation attempt first consumes a fresh qualified `eth_getTransactionCount(sender, "pending")`. Virgin lineage requires that pending equal the activation record's qualified current-chain finalized sender-nonce floor, then allocates that pending value. Later lineage uses `local_high_water + 1` when pending is equal or behind. Any first-use inequality and any later provider-ahead value return typed `NonceLineageDiverged` without mutation. |
| Incomplete nonce reservation | An authenticated issuer plus bounded caller token derives a stable domain-global `SubmissionIntentId`; reservation, candidate, and completion keys exclude `run_id`, implementation-contract refs, and physical generations. One nonce domain permits one incomplete intent. Same ID plus exact intent reuses its nonce across runs; changed semantics conflict; another intent receives typed `NonceDomainBusy`; no timeout, higher-nonce skip, or implicit transfer exists. |
| Candidate convergence | The admitted intent freezes one bounded mutation-equivalent candidate family. The authority serializes a permanent contiguous activated prefix and current ordinal; a closed initial/replacement permit proves every step. Later runs observe every activated hash and reproduce the current member; an older racing member remains a semantically valid winner. |
| Signing convergence | A stable semantic signer fixes public key/address and deterministic RFC 6979 low-`s` behavior across qualified physical generations. Signer attestation records the expected hash before activation; broadcast reproduces and verifies it without retaining signature or bearer bytes. |
| Status and reconciliation | Status is one linearizable current-lineage aggregate snapshot. Injected pure states bind its reservation/completion into current-run producers. A bounded failure linearizes at its last status snapshot; a completion after that point remains visible to a later run and does not retroactively change the earlier run. |
| Terminal convergence | Completion compares a run-independent canonical inclusion-block outcome, not producer-bound witness or later finality-head identity. Compatible later witnesses return the original completion; a conflicting canonical claim is an integrity fault. Purpose-limited completed status returns the canonical object closure needed by a later run's pure projection. |
| Wallet lineage | The nonce namespace is qualified physical chain instance plus sender. A deployment-qualified one-to-one chain registry binds chain ID, genesis, never-reused namespace, and finalized fork anchor so clones remain distinct and redundant routes converge. A separate permanent activation registry uses the wallet domain as primary key and a globally unique activation-record identity to bind exactly one store lineage/activation; a separate store-lineage primary key owns the one current-incarnation head shared by all domains on that lineage. Domain issuance atomically validates/creates that lineage head and inserts the domain row under the same lineage serialization point, returning one composite proof. Every status read carries that stable permanent binding and atomically opens its snapshot through the sealed live target session, every mutation additionally exercises a fresh transaction-bound external-fence permit, and restore/promotion preserves the post-quiescence complete prefix. |
| Wallet-authority upgrades | An incomplete intent freezes its exact semantic contracts and blocks an incompatible release on that sender until the old qualified release completes it. Compatible implementation/physical upgrades preserve the one current schema and complete readable durable lineage. Every incompatible persisted-schema or issuer-namespace change uses a new sender/domain; scalar reset, migration reader, and compatibility interpreter are forbidden. |
| PostgreSQL placement | The domain port and canonical request/evidence types live with EVM. The dedicated narrow `mfm-storage-evm-postgres` adapter owns both the separately credentialed append-only activation-registry admin CAS/deployment proof reader/offline verifier and the per-lineage nonce schema/adapter. Deployment owns the non-rollback external target-fence issuer; Runtime and normal request execution receive no registry role/pool, fence issuer, or registry-admin authority, and ordinary nonce access performs no registry IO. The generic RunHistory store remains EVM-neutral and receives no nonce authority. |
| Telemetry | Logs, metrics, and spans are redacted best-effort observers. Required audit or business acknowledgement is an explicit state, fact, or effect and therefore semantic. |
| Legacy deployment | Activation is a maintenance cutover: stop old admissions, attest every old allocation/submitted candidate terminal with no unresolved possible entry, fence every replay ingress and sender path, bind a qualified finalized-block sender-nonce floor, rotate to a new issuer/idempotency namespace, and retain the old database only as opaque offline audit. The current tree has no old-schema reader, decoder, migration, or certifier. A reused sender starts a virgin current-schema lineage only when fresh pending equals that floor; inability to prove terminality, replay exclusion, floor provenance, or exhaustive fencing requires a new sender/domain. |

The exact field names of the closed EVM submission-failure variants, canonical sentinel bytes,
stable semantic-key encodings, and generated schema hashes are implementation outputs constrained
by these decisions. They do not reopen ownership, recovery, or scheduling policy.

## Material Uncertainties

none
