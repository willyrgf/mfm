# Fix implementation plan: simplify Operation authoring and retire cutover machinery

Status: implementation handoff; the implementation at `af4574f7` remains unapproved until this
plan is complete

Audience: engineer-agents and independent architect reviewers repairing the implemented
[`RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`](RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md)

---

## 0. Authority, base, and stopping rule

The semantic-name and Operation-authoring cutover is implemented by:

```text
f6b62e35  name surviving evm and portfolio contracts
af4574f7  centralize operation expansion and capability injection
```

This plan is a deletion-first correction of that implementation. It supersedes the performance,
private-lowering, test-completion, and cutover-scanner portions of
[`IMPL_PLAN_RFC_FOLLOWUPS.md`](IMPL_PLAN_RFC_FOLLOWUPS.md). The implemented RFC continues to own
the public Operation DSL, structured Match/failure semantics, capability-injection semantics, and
unchanged Program v2 wire. This plan does not reopen those decisions.

The reviewed implementation base is exactly:

```text
af4574f7babdaa9ea79d9886c313216d236f0bb2
```

Relevant reviewed inputs at that base have these SHA-256 values:

```text
RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md
  4dae0e2d91bb30244eaa31c748909f6fd9cb6be9bcf4fc25ffcd7ab5fbdda907
IMPL_PLAN_RFC_FOLLOWUPS.md
  b47a35a83b7765858c63bef052c4ee0f614143cc584d5af7299cf68b7ba2e569
crates/kernel/program/src/authoring.rs
  d0b9338efba7beb31029aacdfda51995820886a87e1284ad5a065b50b227c27b
RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md
  d13d1bfe761931508c4079b30bd47f2eeb3ebac5da2992f92a3ae073d5531e3d
scripts/check-cutover-manifest.sh
  6db84224fa92b34260d85405be898a701fe24a456cbf08eae99ffded72a4fb82
```

If any reviewed input/code changes before implementation—other than adding this exact plan—first
reproduce Section 2 and re-audit every path and line count. Do not silently carry these
measurements onto another tree.

Use four new follow-up commits. Do not rewrite the two reported commits: they may already have
been shared, and history rewriting is unnecessary to make the final tree coherent. Do not add a
compatibility path, feature flag, alternate authoring implementation, or temporary scanner.

Publish this reviewed plan in a documentation-only commit before starting Commit 1. That commit is
not one of the four implementation commits. Implementation metrics always exclude exactly
`FIXES_IMPL_PLAN_RFC_FOLLOWUPS.md`, so the requested handoff document cannot disguise either code
growth or deletion. `af4574f7` remains the semantic/code comparison base.

If the 64-source performance envelope in Section 6.5 is missed after the two identified repeated
work classes are removed, stop. Capture a profile, identify the next measured owner, amend this
plan, and obtain architect approval. Do not add a global cache, another registry, eager
precomputation pass, or domain-owned type-ref table speculatively.

### Material uncertainties

1. **Residual planning cost after the two measured duplication classes are removed**
   - **Choice:** add one expansion-local identity memo and pass each Portfolio collection's one
     checked EVM binding into its configured child Operation.
   - **Why uncertain:** the reproduced `114.13s` maximum-plan run was not accompanied by a
     symbol-level profile, so the exact percentage attributable to repeated schema/identity work
     versus repeated target canonicalization is not known.
   - **Consequence if wrong:** the implementation can be structurally simpler yet still miss the
     maximum-plan latency envelope.
   - **Resolution:** benchmark after Commit 1 and Commit 2 on the same warmed Nix tree. A final
     median above `10s` remains `BLOCK`; profile before approving any further optimization.

2. **Trusted direct callback calls after deleting the scanner**
   - **Choice:** delete the manifest and scanner completely and make direct callback composition a
     reviewed trusted-code rule, like direct State implementation behavior.
   - **Why uncertain:** Rust intentionally permits downstream implementations of the open
     `Operation` and `CapabilityInjection` traits to call their public trait methods directly.
     Compile-fail tests cannot prohibit an in-repository call site.
   - **Consequence if wrong:** a future reviewed-production author could bypass compiler-owned
     child/policy depth accounting until code review catches it. The final `Program::new`
     validator still protects persisted graph structure, and injection remains non-authorizing.
   - **Resolution:** amend the current RFC as specified in Commit 4, record the trust rule in
     rustdoc/current architecture docs, and rely on owner-focused tests plus review. If mechanical
   sandboxing later becomes a product requirement, redesign the open trait; do not restore a
   spelling scanner.

3. **Bounding the trusted App target-descriptor slice**
   - **Choice:** reject more than `EVM_BALANCE_SOURCE_LIMIT` (`64`) target descriptors as
     `ApplicationError::Internal` before cloning inputs for `spawn_blocking`.
   - **Why uncertain:** `plan_snapshot` previously accepted an arbitrarily large sorted target
     slice even though a valid Portfolio plan can use at most 64 nonempty-source collection
     occurrences; an embedder could have supplied one global registry containing unused targets.
   - **Consequence if wrong:** that embedder must pass the bounded relevant subset instead of its
     whole registry.
   - **Resolution:** inventory the current trusted composition callers, amend the RFC/App contract,
     and add the 65-target/zero-Runtime-entry regression in C2-C. If a real caller requires an
     unbounded registry, stop and design an owned bounded selection input; do not clone an
     unbounded slice on the async worker.

No persistence, Runtime, Store, or security uncertainty remains in this repair.

## 1. Target outcome and hard constraints

The final tree must have all of the following properties:

1. `Operation`, `OperationExpansion`, `MatchJoin`, `CapabilityInjection`, `InjectionWriter`, and
   `expand_program` remain the exact public authoring surface.
2. No public callable, type, error, constructor, package, lockfile dependency, or internal MFM edge
   is added. App's existing workspace Tokio dev-dependency is moved to normal dependencies solely
   for `spawn_blocking`.
3. Every stable State/capability/value identity and every current canonical Program/C0 byte remains
   exact.
4. Within pre-Program expansion, one `expand_program` call has at most one successful top-level
   memo miss for each authoring-requested value, State, or capability Rust `TypeId` and category.
   Descriptor construction may recursively derive nested metadata, and `Program::new` retains its
   independent framework-`Never` validation.
5. The memo is private, uniquely owned, lookup-only, and destroyed before `Program` construction.
6. Operation callbacks, Match/handler callbacks, injection hooks, and binding callbacks still run
   once per entered occurrence; none is memoized.
7. The Portfolio planner derives one binding per configured collection child, shares that exact
   `ContentRef` with the child's C0 demand and `CollectEvmBalances`, and the Operation stores only
   that binding.
8. `Application::start_portfolio` keeps borrowed inputs and Runtime-only stored state, but clones
   its bounded secret-free planning inputs and performs synchronous planning in `spawn_blocking`;
   more than 64 target descriptors reject before cloning or Runtime entry.
9. The private draft contains no redundant entry field, root-input mirror, duplicate final graph
   validator, or per-State heap allocation added solely to silence Clippy.
10. A failed Match arm or failure-handler assembly cannot partially mutate its semantic parent
   draft. Fixed bounds and contracts are checked before logical commit; ordinary allocation follows
   the repository's process-allocation policy rather than pretending to be recoverable.
11. Retained-wire hostility is tested through public `Program::decode_canonical`; private raw
    Program validation remains in the Program crate.
12. Callback-scoped authoring values cannot escape and the deliberately absent Effect surface
    remains compile-fail tested.
13. `RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md`,
    `scripts/check-cutover-manifest.sh`, the `negative-scan` task/CI edge, and their task-only tools
    are absent without replacement.
14. CI has nine leaf tasks, not ten, and current workflow documentation describes those nine.
15. Workspace members remain `18`, normal internal MFM dependency edges remain `45`, and public
    mechanical declarations do not exceed the current `197`.
16. The implementation diff, excluding exactly this requested plan document, is materially
    net-negative and meets the exact gates in Section 10; unfiltered repository churn is still
    reported separately.

Do not implement:

- a global, thread-local, process-lifetime, `OnceLock`, or Runtime identity cache;
- `Rc<RefCell<_>>`, `Arc<Mutex<_>>`, interior-mutability cache sharing, or a cache registry;
- cache entries for callbacks, setup values, bindings, hook results, declarations, or errors;
- a second Program validator, alternate draft, prepared-merge object, rollback log, or allocator
  harness;
- domain-owned tables of State/value/capability refs;
- `EvmReadSetup` or another public setup wrapper;
- a new authoring error or error variant;
- a test-only public raw Program constructor;
- a replacement negative scanner, tombstone list, generated manifest, source parser, or API
  snapshot;
- a placeholder `effect` method or Effect trait; or
- any Runtime, Journal, Store, PostgreSQL, CLI/REST, signing, or keystore change, or any App
  production change beyond the exact `spawn_blocking` boundary in Commit 2.

## 2. Reproduced defects and frozen baselines

### 2.1 Correctness and complexity findings

The public design and domain graph are correct, but the implementation is not yet an acceptable
minimal result:

- `crates/kernel/program/src/authoring.rs` is `1,102` lines.
- The selected Program/EVM/Portfolio authoring implementation is `1,248` lines, compared with the
  `327` deleted raw-domain-authoring lines (`+921`).
- A one-collection 64-source/518-declaration plan takes approximately `114.13s` at the final tree
  versus `5.01s` at its parent (`22.8x`).
- The representative two-source plan takes approximately `3.87s` versus `0.44s` (`8.8x`).
- Planning runs synchronously in `Application::start_portfolio` before its first await, so the
  maximum valid request can occupy an async worker for almost two minutes.
- Each repeated Read re-derives expanded/state/capability/value identities and deep schema
  descriptors. Match authoring derives selector and payload descriptors again.
- Each EVM Read calls `EvmPhysicalTarget::binding_ref`, recanonicalizing the same target up to 384
  times for 64 sources.
- `OperationExpansion::input_contract_ref`, `ExpansionDraft::entry`, root-final scans, and several
  `Never` derivations duplicate facts already owned by draft layout or `Program::new`.
- `DraftDeclaration::State(Box<DraftState>)` adds one allocation per State solely in response to a
  private-enum lint.
- Match arm assembly mixes semantic validation, mutation, and a partial `try_reserve` policy even
  though descriptor construction, cloning, hashing, and encoding use ordinary allocation.

The following implementation aspects are already correct and must not be redesigned:

- one flat private draft and one final Program construction;
- child scope reopening;
- structured Match join/terminal inference;
- exact failure partitioning, including nested same-failure fallback;
- before/original/after injection order and original occurrence ownership;
- exact callback-depth bound;
- native/token topology and Portfolio failure mapping;
- all six raw Program constructors being crate-private; and
- the exact five public concepts, root function, and eleven callable items.

### 2.2 Test-boundary findings

The existing tests prove much of the DSL and must be retained. The smallest missing proof set is:

- one top-level identity-cache miss per directly requested type/category across root, child, Match,
  handler, and injection scratch;
- semantic catch/retry after a Match branch reaches a wrong join;
- delegation of root-final contract validation to `Program::new`;
- callback-reference non-escape and the full opaque `OperationExpansion` boundary;
- public-decoder ownership of malformed/noncanonical/unknown/oversized/retired-wire cases; and
- exact representative Portfolio Program and C0 bytes, not only their digest/length; and
- the compact uncovered child/Match/handler/injection rows from RFC §12.2 assigned to C3-D.

Do not recreate a large speculative matrix. Existing aggregate authoring tests already cover
callback depth, empty/invalid root, repeated configured children, nonempty injection, hook
short-circuiting, generic external/adjacent selectors, physical arm order, tag sorting,
unsupported selector shapes, child terminal reopening, recovering/terminal handlers, nested
same-failure behavior, and atomic callback errors.

The raw Program validator already owns the exact `256` accepted / `257` rejected Match-arm ceiling
inside `crates/kernel/program/src/tests.rs`. A source-authored 256-arm selector cannot be created
under the existing 65,536-byte schema-identity limit. Keep the public authoring test at a large
representative selector (`128`) and test the schema-limit rejection. Do not enlarge a schema,
change an error class, or add a second compact selector representation merely to duplicate the raw
Program ceiling.

### 2.3 Scanner finding

The two migration artifacts contain `1,333` lines:

```text
RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md  789
scripts/check-cutover-manifest.sh          544
```

They verify historical deletions and their own 326-entry/83-rule inventory. They are not semantic
validation, can be bypassed by equivalent renaming, and obstruct legitimate future designs. The
only current executable/documentation owners are `nixfied.nix` and
`docs/build-and-verification.md`. No catch-all replacement is allowed.

Current invariants remain owned by Rust visibility/trybuild, Program retained-wire tests, Runtime
association/fold tests, Journal/Store/PostgreSQL contract tests, and domain/App integration tests.

### 2.4 Frozen identity and metric baseline

Preserve:

```text
workspace members                     18
normal internal MFM dependency edges  45
source LOC                         19,793
external-test LOC                   6,905
mechanical public declarations        197
Program authoring.rs lines           1,102
selected authoring lines             1,248
representative Program bytes        36,560
representative Program digest
  content:sha256-v1:59c503d1d8faa1a7afc33d9bb0bcad42d053e7cecb8571023bbc0cef42ab4beb
Program schema
  schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c
one collection / 64 sources             518 declarations
65 sources                              rejected
```

The reported implementation range ledger is:

```text
EVM        crates/domains/evm/src/lib.rs        13-16, 1909-1981
Portfolio  crates/domains/portfolio/src/lib.rs  10-17, 995-1028, 1031-1057
```

Recompute final ranges after `rustfmt`; do not reuse these numbers after code moves.

## 3. Review protocol

Every chunk in Sections 5 through 8 requires a non-author architect review before its commit is
created. A chunk review is path-scoped but examines the complete staged tree so cross-chunk
invariants cannot be hidden.

For each review, record outside the repository:

```text
base commit
staged tree oid (`git write-tree`)
chunk id and exact paths
reviewer
APPROVE or BLOCK
correctness findings
dexterity/future-change-site findings
simplicity/deletion/LOC findings
verification inspected
resolution of every prior BLOCK
```

Every architect is explicitly instructed to minimize concepts, code paths, public types, repeated
derivations, allocations, future change sites, and production LOC. They must `BLOCK`:

- a second identity/descriptor derivation authority;
- shared/global/interior-mutability caching;
- cache-dependent Program bytes or iteration order;
- cached callback/hook/setup/binding behavior;
- duplicated `Program::new` validation;
- a new public item, package/internal/Nix dependency edge, error variant, or alternate authoring
  path (the one App Tokio dev-to-normal role change is explicitly approved);
- domain knowledge moved into Program;
- a positive production delta not justified line by line;
- a missed performance envelope;
- a scanner replacement; or
- removal/weakening of a semantic owner test merely to reduce LOC.

After all chunks in a commit pass, a separate non-author commit architect reviews the integrated
commit. After all four commits, a cumulative architect compares the final tree to
`af4574f7`, verifies the full deletion/metric evidence, and approves the exact candidate that runs
final CI. No permanent approval log or generated evidence file is added to the repository.

## 4. Ordered commits

Implement exactly these four lower-case commits in order:

1. `simplify and memoize operation expansion`
2. `derive evm bindings and isolate portfolio planning`
3. `complete operation authoring boundary proofs`
4. `remove completed cutover scanner`

Each commit must compile and pass its focused checks. Do not squash production repair, test
boundary migration, and migration-tool deletion together; their rollback and review owners are
different.

## 5. Commit 1 — `simplify and memoize operation expansion`

### 5.1 Scope

Primary production paths:

```text
crates/kernel/program/src/lib.rs
crates/kernel/program/src/authoring.rs
```

Primary test path:

```text
crates/kernel/program/tests/authoring.rs
```

No other production crate changes in this commit.

### 5.2 C1-A — one canonical nominal-facts derivation owner

Refactor the existing public helper so the descriptor and reference are derived by one private
owner:

```rust
pub(crate) fn derive_nominal_contract<T: MfmValue>()
    -> Result<(ContentRef, SchemaDescriptor)>
{
    let descriptor = T::schema_descriptor()
        .map_err(|_| ProgramError::InvalidContract)?;
    let schema_id = descriptor
        .identity()
        .schema_id()
        .map_err(|_| ProgramError::InvalidContract)?;
    let semantic_id = T::semantic_id()
        .map_err(|_| ProgramError::InvalidContract)?;
    if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_id) {
        return Err(ProgramError::InvalidContract);
    }
    let contract_ref = ContentRef::new(
        schema_id,
        raw_content_digest(b"mfm.contract.v1"),
    )
    .map_err(|_| ProgramError::InvalidContract)?;
    Ok((contract_ref, descriptor))
}

pub fn nominal_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    derive_nominal_contract::<T>().map(|(contract_ref, _)| contract_ref)
}
```

The exact helper name may change only to avoid a local collision. There must be one implementation
of descriptor/semantic-owner agreement, not one public path plus a copied cache path.

Architect C1-A checks:

- public behavior/error classification is unchanged;
- descriptor/semantic validation exists once;
- the descriptor is retained without re-derivation;
- no value bytes or persisted identities change; and
- no new public item appears.

### 5.3 C1-B — one uniquely owned expansion-local identity memo

Add exactly these two private concepts in `authoring.rs`:

```rust
use std::any::TypeId;
use std::collections::HashMap;

struct CachedValueContract {
    contract_ref: ContentRef,
    descriptor: SchemaDescriptor,
}

#[derive(Default)]
struct IdentityMemo {
    values: HashMap<TypeId, CachedValueContract>,
    states: HashMap<TypeId, ContentRef>,
    capabilities: HashMap<TypeId, ContentRef>,
}
```

Their exact responsibilities are closed:

- `values`: one successful top-level checked nominal contract plus descriptor per authoring-requested
  `MfmValue` Rust type;
- `states`: one successful implementation reference per `State` Rust type; and
- `capabilities`: one successful contract reference per `ReadCapabilityContract` Rust type.

Implement private lookup methods with `TypeId::of::<T>()`. Cache successful results only. Return
cloned `ContentRef`s where ownership is needed; inspect the retained descriptor in place for
selector/payload checks. Do not clone a deep descriptor per lookup.

Conceptual shape:

```rust
impl IdentityMemo {
    fn value<T: MfmValue>(&mut self) -> Result<&CachedValueContract> {
        let key = TypeId::of::<T>();
        if !self.values.contains_key(&key) {
            let (contract_ref, descriptor) = derive_nominal_contract::<T>()?;
            self.values.insert(
                key,
                CachedValueContract {
                    contract_ref,
                    descriptor,
                },
            );
        }
        self.values
            .get(&key)
            .ok_or(ProgramError::InvalidContract)
    }

    fn value_ref<T: MfmValue>(&mut self) -> Result<ContentRef> {
        self.value::<T>().map(|facts| facts.contract_ref.clone())
    }

    fn state_ref<S: State>(&mut self) -> Result<ContentRef> {
        // Cache state_implementation_ref::<S>() by TypeId::of::<S>().
    }

    fn capability_ref<C: ReadCapabilityContract>(&mut self) -> Result<ContentRef> {
        // Cache capability_contract_ref::<C>() by TypeId::of::<C>().
    }
}
```

The two abbreviated methods must use the same explicit no-`unwrap` insertion/read pattern. Do not
introduce a key-kind enum, heterogeneous `Any` map, cache trait, or generic registry.

`expand_program` constructs exactly one `IdentityMemo` and obtains the root input, output, and
failure references through it before constructing `OperationExpansion`. `OperationExpansion`, `MatchJoin`, and
`InjectionWriter` each own it while their callback is active. Transfer it into a nested scratch
with `std::mem::take`, then restore it before propagating the callback result:

```rust
let mut identities = IdentityMemo::default();
let input = identities.value_ref::<O::Input>()?;
let output = identities.value_ref::<O::Output>()?;
let failure = identities.value_ref::<O::Failure>()?;
let mut expansion = OperationExpansion::new(
    input.clone(),
    output.clone(),
    failure.clone(),
    Vec::new(),
    1,
    identities,
);
```

```rust
let mut child_expansion = OperationExpansion::new(
    input,
    output,
    failure,
    admitted_failures,
    depth,
    std::mem::take(&mut self.identities),
);
let callback_result = child
    .expand(&mut child_expansion)
    .map_err(authoring_error);
self.identities = std::mem::take(&mut child_expansion.identities);
callback_result?;
```

Use that ownership pattern for:

- child `Operation::expand`;
- Match definition and every arm;
- protected and handler callbacks; and
- before/binding/after Read occurrence construction.

There must be no `?` or early return between a nested callback returning and restoration of the
memo. Capture the result, restore first, then classify/propagate it and perform every
post-callback validation or merge. The root callback already borrows the one expansion that owns
the memo; capture its result before `?`, but no transfer/restoration is needed there.

For Read construction, prefer one private consuming helper rather than repeating error restoration:

```rust
fn expand_read_suffix<S, C>(
    setup: &C::Setup,
    identities: IdentityMemo,
    /* exact active contract/depth inputs */
) -> (IdentityMemo, Result<ExpansionDraft>)
where
    S: ReadState<C>,
    C: ReadCapabilityContract + CapabilityInjection<S>,
{
    let mut writer = InjectionWriter {
        draft: ExpansionDraft::new(/* expanded input */),
        required_failure_contract_ref: /* S::Failure */,
        identities,
    };
    let result = (|| {
        <C as CapabilityInjection<S>>::write_before(setup, &mut writer)
            .map_err(authoring_error)?;
        // Check S::Input, then invoke binding exactly once.
        let binding_ref = <C as CapabilityInjection<S>>::original_binding_ref(setup)
            .map_err(authoring_error)?;
        // Only after binding succeeds derive S, C, Intent, and Evidence identities;
        // append the designated original exactly once.
        <C as CapabilityInjection<S>>::write_after(setup, &mut writer)
            .map_err(authoring_error)?;
        // Check C::ExpandedOutput and all suffix-local invariants.
        Ok(())
    })();
    let InjectionWriter { draft, identities, .. } = writer;
    (identities, result.map(|()| draft))
}
```

Use exact UFCS (`<C as CapabilityInjection<S>>::...`) where capability policies for several
States would make `C::...` ambiguous.

The memo is never part of `ExpansionDraft`, `Program`, canonical bytes, decode, Runtime, or a
domain value. A failed scratch may return successful deterministic identity facts to its parent;
that is invisible memoization, not graph mutation.

Preserve lazy ordering. In particular, Read construction remains:

```text
expanded input/output and State input/output/failure facts
parent current-contract check against expanded input
State-failure legality check against the scope/admitted failures
first nested-callback-depth check
write_before
State-input continuity
second nested-callback-depth check
original_binding_ref
State implementation ref
capability contract ref
Intent ref
Evidence ref
designated original append
third nested-callback-depth check
write_after
expanded-output and suffix validation
```

Therefore:

- do not prewarm all identities;
- do not derive the binding or State/capability/Intent/Evidence identities before a
  `write_before` that currently fails first;
- do not derive State/capability/Intent/Evidence identities when the binding callback fails;
- do not invoke `write_after` after a prior failure; and
- do not cache a callback result or an identity error.

Use the memoized `Never` value reference throughout authoring. Pass it into append/failure helpers;
delete every helper-local `never_ref()` re-derivation.

After the cutover, `authoring.rs` has no direct call to `nominal_contract_ref` or `never_ref`, and
calls `derive_nominal_contract`, `state_implementation_ref`, and `capability_contract_ref` only
inside the corresponding `IdentityMemo` miss path. All other top-level authoring lookups go through
the memo. The public helpers remain for external callers and
`Program::new` keeps its independent internal validation. Verify this once during review; do not
add a recurring source scanner.

Refactor selector/payload checking so Match occurrences retain no cloned `SchemaShape`. A Match
scratch keeps its selector `TypeId`; its compact mutable ledger keeps only tag, descriptor ordinal,
and `seen`:

```rust
struct SelectorVariant {
    tag: String,
    descriptor_ordinal: usize,
    seen: bool,
}

fn selector_variants(
    selector: &CachedValueContract,
) -> Result<Vec<SelectorVariant>>;

fn require_payload<P: MfmValue>(
    identities: &mut IdentityMemo,
    selector_type_id: TypeId,
    descriptor_ordinal: usize,
) -> Result<()>;
```

`require_payload` first ensures `P` has one memo entry, then compares that cached descriptor with
the exact payload descriptor reached by ordinal inside the memo-held selector descriptor. It must
not call `P::schema_descriptor`, `P::semantic_id`, or `nominal_contract_ref::<P>()` again and must
not clone a selector/payload `SchemaShape`. HashMap iteration order never affects arm order or
canonical bytes.

Architect C1-B traces every callback/error path and `BLOCK`s any lost memo on ordinary error,
changed callback ordering, `Rc`/`RefCell`, cache behavior beyond one root expansion, or output
dependence on HashMap iteration.

### 5.4 C1-C — delete redundant private lowering state

After the memo is wired, reduce the compiler rather than layering it over all current checks.

Delete `OperationExpansion::input_contract_ref`. It is an immutable mirror compared only with the
value used to construct the root. The final field set is:

```rust
pub struct OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    admitted_failure_contract_refs: Vec<ContentRef>,
    draft: ExpansionDraft,
    identities: IdentityMemo,
    callback_depth: u8,
    marker: PhantomData<fn(I) -> (O, F)>,
}
```

Delete `ExpansionDraft::entry`. Every nonempty scratch enters at declaration zero before rebasing;
an unconnected merge enters at its append offset. Replace entry bookkeeping with:

```rust
fn first_contract(&self) -> Result<&ContentRef>;

fn is_state_first(&self) -> bool {
    matches!(self.declarations.first(), Some(DraftDeclaration::State(_)))
}
```

`merge_connected` compares the parent frontier with `scratch.first_contract()`.
`merge_unconnected` returns `DraftId(offset)` as the detached entry. Delete entry initialization,
mutation, rebasing, and `Option` checks.

Unbox the State-heavy private draft:

```rust
// Bounded source-authoring drafts are State-heavy; contiguous storage avoids one allocation per State.
#[allow(clippy::large_enum_variant)]
enum DraftDeclaration {
    State(DraftState),
    Match(DraftMatch),
}
```

Do not add a wrapper solely for the lint. Keep the allowance on this one private enum. Record the
reviewed layout arithmetic rather than inventing an RSS harness: the current boxed draft is about
`441 KiB` plus `454` State allocations at the 518-declaration maximum, while the unboxed bounded
vector is about `390 KiB` and has no per-State allocation. Re-measure `size_of` values if the fields
change materially; do not commit a layout test.

Delegate only root-final graph validity to the existing authoritative validator. Root completion
becomes conceptually:

```rust
let root_result = root.expand(&mut expansion).map_err(authoring_error);
root_result?;
let OperationExpansion {
    mut draft,
    identities,
    ..
} = expansion;
drop(identities);
draft.seal_open_success()?;
draft.seal_open_failures()?;
let declarations = draft.into_declarations()?;
Program::new(entry_point_id, input, output, failure, declarations)
```

Freeze `seal_open_success` as a route operation, not another contract validator:

```rust
fn seal_open_success(&mut self) -> Result<()> {
    if let Some(frontier) = self.open_success.take() {
        self.patch_success_tails(&frontier.tails, DraftRoute::Terminal)?;
        self.scope_success.extend(frontier.tails);
    }
    Ok(())
}
```

It accepts an empty frontier and does not compare output contracts or require a State. This is
intentional: `Program::new` alone decides whether a zero-State root and every terminal output are
valid. The helper only eliminates unresolved source routes before lowering.

Delete:

- the root-input self-comparison;
- the duplicate empty-root branch;
- `require_failures` and its scans (all State append paths already enforce exact active/admitted
  failure contracts);
- `require_resolved` (lowering rejects `Open`, then `Program::new` validates the graph);
- root-success output/terminal scans duplicated by `Program::new`;
- the redundant `require_current(output)` immediately after successful
  `reopen_scope_success(output)`; and
- source-index forward checking in `lower_target` after checked conversion.

Keep DSL-local checks that `Program::new` cannot reconstruct: current typed frontier, child input/
output/failure scope, supported Match descriptor/payload, arm join/terminal classification,
handler admission/partition, injection continuity, callback depth, fixed capacity checks, and
detached-scratch semantic atomicity.

Architect C1-C compares each deleted check to an exact `Program::new` owner. A deletion is allowed
only when the final validator or an earlier append/scope invariant owns the same failure.

### 5.5 C1-D — delete the partial allocator-recovery policy

Delete authoring-specific `try_reserve` and do not add `reserve_match_variant`, `PreparedMerge`, an
allocation transaction, rollback log, allocator harness, or `unsafe`. Descriptor construction,
HashMap insertion, cloning, hashing, canonical encoding, and ordinary `Vec` growth already follow
the process allocator. Treating just one frontier extension as recoverable `ProgramError::Capacity`
is extra code without a coherent guarantee.

Preserve all deterministic capacity contracts:

- callback depth;
- declaration, State, Match-arm, and `u16` bounds;
- checked count and rebase arithmetic; and
- the final canonical Program byte ceiling in `Program::new`.

These explicit limits continue to return `ProgramError::Capacity`. OOM is not a typed authoring
error contract.

Preserve semantic atomicity with the existing detached scratch ownership. A child, protected body,
handler, or injection suffix is local and drops on error before its parent is touched. A
`MatchJoin` is callback-reusable after an arm returns `Err`, so its arm path must perform, in order:

1. validate tag membership/unseen status, cached payload/input compatibility, and callback depth
   without setting `seen` or invoking the callback;
2. build the branch in a detached scratch;
3. validate nonempty/State-first entry, join/terminal output, failure routes, and all fixed count/
   rebase bounds;
4. merge the semantically valid branch with ordinary `Vec` growth; and
5. append the variant metadata and set `seen`.

There must be no typed semantic/capacity failure after step 4 begins. An allocator abort is outside
the `Result` contract. Failure-handler assembly needs no analogous transaction: both callbacks and
the combined handler graph remain in one detached composite until the whole public method
succeeds. Validate its contracts, fixed bounds, handled tails, and all ordinary local/cross-scratch
targets before merging that composite into the parent.

Architect C1-D traces Match arm retry and handler parent isolation, confirms every fixed limit still
has one checked owner, and rejects any attempt to reintroduce partial recoverable-allocation
machinery.

### 5.6 C1-E — focused regressions and deterministic performance proof

Add manual counter types in `crates/kernel/program/tests/authoring.rs` and prove:

1. a repeated top-level authoring request for one value `TypeId` causes one memo miss and one
   `schema_descriptor` call per `expand_program`;
2. a manual derive-shaped counter value whose `schema_descriptor()` deliberately invokes
   `Self::semantic_id()` once (matching the derive expansion) performs exactly two semantic calls
   per miss—descriptor construction plus nominal-owner agreement—not per occurrence;
3. nested metadata derivation inside a descriptor is not counted as a separate cache promise;
4. a repeated State type invokes `state_id` once and a repeated Read capability invokes
   `contract_id` once;
5. the same directly requested types reused across a child, Match arm, failure handler, and
   injection support suffix share the same memo;
6. two separate `expand_program` calls each perform their own first derivation;
7. an errored scratch can populate only invisible memo entries and leaves Program bytes equal to a
   clean retry;
8. a Match arm that reaches the wrong join returns `InvalidContract`, can be caught, and the same
   tag can then be authored correctly with bytes equal to a clean Match; and
9. a nonempty root with the wrong terminal output is rejected by `Program::new` after duplicate
   root checks are deleted.

Retain the existing callback/hook occurrence-count tests. Strengthen them only if the memo refactor
breaks their ability to distinguish before/binding/after calls; do not add a duplicate test merely
to renumber the same callbacks.

Do not add timing assertions to Rust tests. Wall-clock limits are environment-sensitive; the
deterministic call-count regression owns the repeated-work invariant.

Focused verification:

```text
nix develop -c cargo fmt --all -- --check
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test -p mfm-portfolio --test planning_contract \
  representative_program_identity_is_stable -- --exact
```

After compilation is warm, run the exact maximum test three times and record Cargo's test-body
duration:

```text
nix develop -c cargo test -p mfm-portfolio --test planning_contract \
  portfolio_program_and_c0_capacity_contract -- --exact --nocapture
```

This measurement is diagnostic after Commit 1; the final hard envelope is applied after Commit 2.

### 5.7 Commit 1 gate

The Commit-1 architect must report:

- exact top-level memo-miss and identity call counts before/after, with nested descriptor work
  scoped honestly;
- `authoring.rs` line count and production churn;
- public declarations and dependency edges unchanged;
- no `Rc`, `RefCell`, global/static cache, or cached callback result;
- exact representative Program digest/bytes unchanged;
- maximum-plan timing after the core memo; and
- structural proof that no typed Match/handler semantic failure follows logical commit mutation.

Target `authoring.rs <= 1,102` lines. A result up to `1,142` is allowed only if the reviewer gives a
line-level account of irreducible cache code and confirms the combined Commit-1 production Rust
delta is no more than `+40`. Above that is `BLOCK`.

## 6. Commit 2 — `derive evm bindings and isolate portfolio planning`

### 6.1 Scope

Primary paths:

```text
crates/domains/evm/src/lib.rs
crates/domains/evm/README.md
crates/domains/portfolio/src/lib.rs
crates/domains/portfolio/tests/planning_contract.rs
crates/domains/portfolio/tests/fixtures/portfolio-program-two-sources-v2.json
crates/domains/portfolio/tests/fixtures/portfolio-c0-two-sources.json
crates/app/Cargo.toml
crates/app/src/lib.rs
crates/app/README.md
crates/app/tests/portfolio_runtime.rs
RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md
```

Runtime/live production code remains unchanged. Existing App/live tests are verification owners;
App production changes only at the blocking-planner boundary in C2-C.

### 6.2 C2-A — store the exact checked binding, not the target

Change only the configured Operation's private representation:

```rust
pub struct CollectEvmBalances<K: MfmValueTrait> {
    binding_ref: ContentRef,
    source_count: usize,
    marker: PhantomData<fn() -> K>,
}

impl<K: MfmValueTrait> CollectEvmBalances<K> {
    pub fn new(
        binding_ref: ContentRef,
        source_count: usize,
    ) -> Result<Self, EvmDomainError> {
        if !(1..=EVM_BALANCE_SOURCE_LIMIT).contains(&source_count) {
            return Err(EvmDomainError::Program);
        }
        Ok(Self {
            binding_ref,
            source_count,
            marker: PhantomData,
        })
    }
}
```

Preserve the public constructor name but deliberately narrow its input to the exact binding needed
by expansion. In `plan_snapshot`, derive `route_ref = target.binding_ref()?` once, clone that
`ContentRef` into the C0 demand, and move/clone the same value into the configured child. Inline EVM
tests derive the ref before constructing the child. Do not retain the target, add a getter, or cache
a binding inside `EvmPhysicalTarget` (that would alter its value contract).

All six exact policies become:

```rust
type Setup = ContentRef;

fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
    Ok(setup.clone())
}
```

Each Read passes `&self.binding_ref`. `original_binding_ref` is still invoked once per occurrence;
it is now a cheap immutable clone rather than repeated canonicalization.

This is an intentional narrow amendment to RFC §6.1/§6.5/§7.1: setup for these six identity
policies is the exact kernel `ContentRef`, not a domain-owned setup product; the trusted Portfolio planner derives the
binding from checked `EvmPhysicalTarget` once, C0 and its child Operation share that value, and the
exact-pair policy consumes it. Direct source authors are already trusted and injection is not
authorization; a supplied binding without an exact trusted-composition registration is rejected by
Runtime assembly before provider entry.

Hoist the two constant Match tags outside the source loop:

```rust
let native = StableId::new("native").map_err(|_| ProgramError::InvalidContract)?;
let token = StableId::new("token").map_err(|_| ProgramError::InvalidContract)?;
for _ in 0..self.source_count {
    // ...
    body.match_join::<EvmBalanceAsset<K>, EvmBalanceContext<K>>(|arms| {
        arms.arm::<EvmBalanceContext<K>>(native.clone(), /* native */)?;
        arms.arm::<EvmBalanceContext<K>>(token.clone(), /* token */)
    })?;
}
```

No declaration, tag order, State/capability ref, or Program byte changes.

Architect C2-A checks that the Operation no longer retains an unused target, the Portfolio planner
derives one binding per configured collection child and shares it with that child's C0 demand, all six policies remain
explicit, no blanket impl or setup wrapper appears, and arbitrary binding content grants no
Runtime authority.

### 6.3 C2-B — domain and identity regressions

Retain and run:

- exact EVM target binding golden;
- native/token/mixed source Program topology;
- two-collection handler/rejoin topology;
- 64 accepted / 65 rejected source capacity;
- representative `36,560` Program bytes/digest;
- wrong chain and wrong route returning pre-provider `RuntimeError::Internal` with zero provider
  calls; and
- hot/cold App equality and provider counts.

Retain `planner_binds_every_read_to_the_exact_target`, which already inspects all twelve Reads for
two sources and proves all six per source carry the exact target binding. Retain the repeated-child
test proving two collection bindings stay distinct. Add no equivalent assertion or getter.

Capture exact no-newline fixtures from `af4574f7` before changing the constructor:

```text
crates/domains/portfolio/tests/fixtures/portfolio-program-two-sources-v2.json
crates/domains/portfolio/tests/fixtures/portfolio-c0-two-sources.json
```

Extend `representative_program_identity_is_stable` to compare the complete authored Program bytes
and `canonicalize_mfm_value(&c0)` bytes with those fixtures. Also hard-code and assert the full C0
`ContentRef` string captured from the base, while retaining the current Program length/ref and
topology diagnostics. This behavior-changing commit—not a later test-only commit—owns proof that
sharing the binding changes no persisted byte or reference. Do not inline the 36,560-byte Program
or duplicate the three public result fixtures.

Update RFC §6.1/§6.5/§7.1 and README wording from `Setup = EvmPhysicalTarget`/domain-owned setup to
the exact derived-binding ownership above. Do not generalize this exception to future policies or
alter the future Effect known gap.

Focused verification:

```text
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task capacity-app
```

### 6.4 C2-C — keep CPU planning off the async worker

In `crates/app/Cargo.toml`, move the existing workspace Tokio entry from `[dev-dependencies]` to
`[dependencies]`. This adds no package, version, feature, lockfile, Nix dependency, or internal MFM
edge.

Keep the public borrowed signature and Runtime-only `Application` state. Apply an O(1) bound before
cloning, then move the bounded secret-free inputs into one blocking task and await it immediately:

```rust
if targets.len() > mfm_evm::EVM_BALANCE_SOURCE_LIMIT {
    return Err(ApplicationError::Internal);
}
let owned_config = config.clone();
let owned_targets = targets.to_vec();
let planned = tokio::task::spawn_blocking(move || {
    plan_snapshot(selector, &owned_config, &owned_targets)
})
.await
.map_err(|_| ApplicationError::Internal)?;
let (program, c0) = planned.map_err(map_portfolio_error)?;
if program.entry_point_id().as_str() != PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID {
    return Err(ApplicationError::Internal);
}
self.runtime
    .start(run_id, program, c0)
    .await
    .map_err(ApplicationError::Runtime)
```

Do not move Runtime admission/progression into the blocking closure, store authoring inputs on
`Application`, change the borrowed public signature, expose a generic blocking helper, or add a
second planning API. `JoinError` is trusted-process failure and maps to the existing redacted
`ApplicationError::Internal`; `PortfolioError` retains the existing explicit mapping.

The target limit is a deliberate narrow App contract change: a valid checked Portfolio plan has at
most 64 total sources, so a larger descriptor slice necessarily contains unused entries. Reject it
as trusted-composition `Internal`, not caller `InvalidRequest`, and do so before cloning, planning,
Store access, or Runtime admission. Amend the RFC and `crates/app/README.md` with this exact bound.
Add one boundary regression: 64 distinct valid targets remains accepted; 65 returns `Internal`
with zero Runtime Store loads/appends. Do not add a second capacity constant; import the EVM source
limit.

Dropping the outer future may leave an already-started blocking task to finish, but that task is
pure, bounded, secret-free authoring with no Store/provider/Runtime entry; its discarded result has
no semantic effect. Existing App start/error/hot-cold tests must remain exact. The C2 architect
inspects the direct `spawn_blocking` boundary; do not add a timing-sensitive async test or a private
injection seam solely to test Tokio scheduling.

### 6.5 C2-D — final performance envelope

Using clean temporary worktrees on the same host and pinned Nix toolchain, warm the exact test once
and then record three measured runs at each of:

```text
f6b62e35  raw-authoring baseline
af4574f7  reported Operation implementation
Commit 2 candidate
```

Use each tree's median Cargo test-body duration. Do not compare a new candidate median against one
historic single sample; the `5.01s` and `114.13s` observations are diagnostic context only.

Acceptance requires:

```text
candidate median <= 10.00s
candidate median <= 2x the same-host f6b62e35 median
candidate median <= 0.1x the same-host af4574f7 median
```

Do not add a benchmark dependency, committed benchmark artifact, RSS harness, or flaky CI timing
assertion. If the candidate misses any bound, Commit 2 remains `BLOCK`. Capture a profile and amend
the plan rather than adding an unreviewed cache layer.

Architect C2-D also checks that the Portfolio planner is the sole target-to-binding derivation owner,
`CollectEvmBalances::new` only stores the supplied ref, and no App/Runtime/live production
workaround was added beyond the exact C2-C blocking boundary.

### 6.6 Commit 2 gate

The Commit-2 architect must approve:

- exact Program/C0 identities;
- exact full Program/C0 fixture bytes and hard-coded C0 `ContentRef`;
- exact six-policy setup/binding mapping;
- 64-target acceptance contract and 65-target pre-clone `Internal`/zero-Runtime-entry rejection;
- no new public item, package, internal edge, or lockfile change; exactly one Tokio dependency-role
  promotion in App;
- domain selected-authoring LOC non-positive relative to the current 146-line ledger;
- final performance envelope; and
- unchanged Runtime/Journal/Store/live production diff and only the reviewed App blocking diff.

## 7. Commit 3 — `complete operation authoring boundary proofs`

This commit moves tests to their correct owners and adds only the small missing regression set. It
must contain no non-test production Rust change; `src/tests.rs` and `src/assembly/tests.rs` are
explicit `cfg(test)` owners.

### 7.1 C3-A — external retained-wire ownership

Paths:

```text
crates/kernel/program/src/tests.rs
crates/kernel/program/tests/retained_wire.rs
crates/kernel/program/tests/fixtures/pure-program-v2.json
crates/kernel/program/tests/fixtures/read-match-program-v2.json
```

Move, do not copy, all `Program::decode_canonical` hostile cases from private `src/tests.rs` into
external `tests/retained_wire.rs`:

- malformed JSON (`b"{"`);
- noncanonical trailing whitespace;
- input above `MAX_RUN_OBJECT_CANONICAL_BYTES`;
- missing and unknown fields;
- unknown/retired declaration tags and fields;
- `u16` overflow;
- hostile Match-arm ordering; and
- both exact full-string retained Program fixtures.

Store the pure and Read/Match canonical bytes as exact `include_bytes!` fixtures with no trailing
newline. Table-test their full bytes, schema ID, and content digest. Remove inline copies, the
decode-only `state_program()` helper, and the decode-only canonical-JSON mutation helper from
`src/tests.rs`. The exact source-authoring encoder proof is the Portfolio Program fixture in C2-B;
do not retain a second private fixture path solely for the pure sample.

After the move:

- private `src/tests.rs` owns raw `Program::new`/constructor graph validation, including the exact
  256/257 Match ceiling; and
- external `retained_wire.rs` owns every public decoder/wire rejection.

Do not duplicate Runtime decoder tests or make the deliberately source-invalid retained
Read/Match fixture source-authorable.

Architect C3-A compares old/new assertion counts and fixtures to prove this is an ownership move,
not a test deletion or a second wire parser.

### 7.2 C3-B — scoped authoring compile-fail boundary

Extend the existing `tests/ui/scoped_authoring.rs` rather than adding a broad API snapshot. Prove:

- `OperationExpansion<Never, Never, Never>` has no struct-literal construction, public `new`,
  `Clone`, or `Default`;
- `MatchJoin` and `InjectionWriter` have no public `new`, `Clone`, or `Default`;
- `OperationExpansion`, `MatchJoin`, and `InjectionWriter` expose none of the forbidden raw-route,
  finalization, child-on-writer, or `effect` methods; and
- the existing `removed_program_api.rs` continues to prove all six raw constructors are private
  and `ProgramAuthor`/`ForwardLabel` absent.

Add exactly one new fixture, `tests/ui/scoped_escape.rs`, plus `.stderr`. It proves three callback
lifetime boundaries:

1. the `&mut OperationExpansion` received by `Operation::expand` cannot be passed to a helper
   requiring a `'static` mutable expansion;
2. a `&mut MatchJoin` received by `match_join` cannot be passed to a helper requiring
   `&'static mut MatchJoin<...>`; and
3. a `&mut InjectionWriter` received by a local test capability hook cannot be passed to a helper
   requiring `&'static mut InjectionWriter`.

All three must fail with the borrow escaping the callback (`E0521` or the compiler's exact equivalent
on the pinned toolchain). Add one explicit trybuild harness line. Do not multiply escape cases or
add an unsupported-pair fixture already enforced by trait bounds.

Architect C3-B checks the pinned `.stderr` against the actual failure and rejects tests that fail
for an unrelated missing import/type error.

### 7.3 C3-C — close only the real association gap

In `crates/kernel/runtime/src/assembly/tests.rs`, extend the retained Match fixture family with one
case that registers only the selector codec, deliberately omits `AsymmetricPayload`, and requires
`RuntimeError::IncompatibleAssembly`. The existing `association_result` helper always registers the
payload, so give this case a narrow setup flag/helper rather than accidentally reusing it unchanged.
Retain the existing unsupported tagging/shape, missing/unknown tag, semantic mismatch,
serialized-shape mismatch, and payload-target mismatch cases.

Do not invent a source-authored mismatch that the typed State registration API cannot construct,
and do not duplicate Program-level empty/duplicate-arm rejection in Runtime. State registration
already installs valid target payload codecs; the private association helper is the correct owner
for an intentionally missing codec.

Architect C3-C checks that Runtime production is byte-identical and the new test reaches
association rather than failing Program decode.

### 7.4 C3-D — complete the frozen structured-authoring matrix compactly

Extend the existing aggregate fixtures in `crates/kernel/program/tests/authoring.rs`; do not create
one test file or bespoke framework per row. Cover the RFC obligations that the reported completion
did not exercise directly:

- child scope: wrong input rejects before callback invocation; wrong output rejects after exactly
  one callback; the 65th callback-depth entry returns `Capacity`, can be caught, and leaves parent
  bytes equal to a clean retry;
- Match: a non-generic adjacent heterogeneous selector, an arm authored by a child `Operation`,
  empty/Match-first/wrong-join arms caught then retried, and an all-terminal Match;
- handler: no reachable handled failure, outer-failure bypass, foreign failure, nested distinct
  handlers, and Match-first/wrong-input/wrong-join/foreign-failure handler rejection; and
- injection: multiple Pure support States plus one support State with exactly the designated
  original's failure contract, whose edge reaches the active nearest handler.

Reuse existing fixture types and table-drive homogeneous failures. Retain existing Runtime
classified-recovery/hot-cold coverage instead of duplicating classifier + Match execution here.
The wrong-join retry is semantic late-failure atomicity; do not label it allocator recovery.

Architect C3-D maps each row to the RFC acceptance clause, rejects redundant cases already proved
at the same boundary, and confirms no production escape hatch was added to make a hostile fixture
constructible.

### 7.5 Commit 3 gate

Focused verification:

```text
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-portfolio -p mfm-app --all-targets
```

The Commit-3 architect confirms:

- no non-test production Rust diff exists (`src/tests.rs` and `src/assembly/tests.rs` are
  `cfg(test)` owners);
- decoder cases moved rather than duplicated;
- each compile-fail fixture fails for its intended visibility/lifetime reason;
- no scanner behavior was reimplemented in Rust tests; and
- exact Program/C0 fixtures match before/after.

## 8. Commit 4 — `remove completed cutover scanner`

### 8.1 C4-A — delete migration artifacts and task graph edges

Delete completely:

```text
RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md
scripts/check-cutover-manifest.sh
scripts/  # directory becomes empty
```

In `nixfied.nix`:

- delete the `negative-scan` task;
- delete the `negative-scan` CI step;
- remove task-closure `pkgs.coreutils`, `pkgs.gawk`, and `pkgs.ripgrep` from `cargoTools`; and
- keep `pkgs.bash`, `pkgs.git`, `pkgs.pkg-config`, the Rust toolchain, compiler, and platform tools.

Do not remove the separate `pkgs.ripgrep` developer-shell tool in `flake.nix`; it is still useful
for development and is not owned by the deleted task.

Do not change `slotPolicy.max = 9`; the remaining CI has nine leaf tasks and the existing bound is
already exact.

The resulting CI composition is:

```text
fmt
clippy
cargo-check
cargo-test
postgres-test (through test-db)
doc-tests
capacity-app
capacity-runtime
capacity-store
```

Architect C4-A verifies the deleted tools have no remaining Nixfied consumer, `model-check`
resolves the task graph, and no shell/source scanner replaces the deleted files.

### 8.2 C4-B — current workflow and RFC authority

Update `docs/build-and-verification.md`:

- remove the `negative-scan` task row;
- remove “absence” from the CI description; and
- keep semantic owner tests and scope-driven verification unchanged.

Update `docs/architecture.md`, `crates/kernel/program/README.md`, and the public authoring rustdoc
in `crates/kernel/program/src/lib.rs` with one concise trusted-code rule: Operation
implementations compose children only through `OperationExpansion`; capability policies emit
support States only through `InjectionWriter`; direct trait-callback calls bypass kernel callback
accounting and are forbidden in reviewed production code. Do not describe this as a security or
authorization boundary.

Amend `RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md` narrowly:

1. Material uncertainty 4 chooses reviewed trusted callbacks, not a repository scanner.
2. The direct-callback section says in-repository production composition is a code-review rule;
   `Program::new` remains the persisted-graph boundary.
3. §10.6 becomes “documentation and owner-test enforcement”; delete the manifest/fingerprint/
   canary requirements.
4. Remove `negative-scan` commands from verification.
5. Acceptance 19 keeps callback-depth and supported-composition semantics without claiming a
   scanner.
6. Acceptance 26 requires current docs and owner tests to teach/prove the current authoring model.

Add one short implementation note near the top of
`RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`: its deletion inventory and negative scanner were
one-time cutover gates; this plan retires their manifest/task after the semantic owners became
tests and current docs. Mark the RFC implemented and replace its stale relationship paragraph that
says the now-implemented follow-up “must later rebase” with the actual implemented relationship.
Do not rewrite its historical deletion lists.

Completed implementation plans are historical execution records. Make status-only edits mandatory
for all three documents so their retained old scanner commands cannot be mistaken for the current
workflow:

```text
IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md
FIXES_TT1_IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md
IMPL_PLAN_RFC_FOLLOWUPS.md
```

Each status says “implemented/historical”, points to this fixes plan and
`docs/build-and-verification.md` for the current repair/workflow, and states that commands below are
the historical execution record. Do not rewrite completed chunks or their old command evidence.

Do not add a tombstone appendix or move the 326 manifest entries into an RFC. Git history is the
deletion record.

Architect C4-B checks that no current document claims the scanner exists, historical plans remain
clearly historical, the trusted-callback consequence is stated honestly, and no Effect symmetry
or security claim changes.

### 8.3 C4-C — deletion evidence

Record one-time handoff evidence; do not commit it as a script or manifest:

```text
deleted file/path check
workspace package/member check
normal internal-edge list
public constructor/alternate-authoring check
current-doc link/command review
```

One-time `rg` inspection is allowed during review. It is evidence, not a recurring gate. Do not
turn the command into a Nix task, shell file, generated allowlist, or CI step.

Focused verification for the task-graph change:

```text
nix run .#model-check
nix flake check --no-build
git diff --check
```

The Commit-4 architect requires an expected diff of roughly `-1,333` lines before task/docs
cleanup and no executable/semantic Rust change; the specified `src/lib.rs` rustdoc-only edit is
allowed.

## 9. Exact deletion and refactor ledger

### 9.1 Delete outright

```text
RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md
scripts/check-cutover-manifest.sh
scripts/
nixfied.tasks.negative-scan
the negative-scan CI step
task-only cargoTools entries: coreutils, gawk, ripgrep
```

### 9.2 Delete from private authoring code

```text
OperationExpansion::input_contract_ref
ExpansionDraft::entry
entry initialization/rebase/mutation branches
require_failures and repeated failure scans
require_resolved and repeated declaration scan
duplicate empty-root validation
duplicate root-input comparison
duplicate root-success/output/terminal scans
redundant require_current after reopen_scope_success
helper-local repeated never_ref derivations
source-index forward validation duplicated by Program::new
Box<DraftState> and one heap allocation per State
authoring-specific try_reserve / partial allocator-recovery plumbing
second selector descriptor derivation
second payload descriptor/semantic derivation
per-occurrence EvmPhysicalTarget canonicalization
per-source StableId validation for constant native/token tags
```

### 9.3 Refactor, do not duplicate

```text
nominal contract derivation -> one private facts function + existing public ref helper
typed identity lookups      -> one root-local IdentityMemo
selector/payload checks     -> cached descriptors
Match frontier merge        -> semantic validate-before-commit + ordinary Vec growth
handler composite merge     -> detached scratch + fixed-bound validation
CollectEvmBalances target   -> planner-owned checked ContentRef binding
private decoder hostility   -> external retained_wire owner
inline retained strings     -> exact no-newline fixtures
```

### 9.4 Explicitly retain

```text
Program v2 and its six crate-private constructors
Program::new as sole full graph validator/encoder
Program::decode_canonical as sole retained-byte ingress
one ExpansionDraft with local DraftId/DraftRoute
all DSL-local continuity/Match/handler/injection checks
all five public authoring concepts and eleven callables
all six explicit EVM injection pairings
exact callback-depth bound 64/65
exact native/token and Portfolio failure topology
Runtime/Journal/Store/PostgreSQL production code
App production code except the exact C2-C spawn_blocking boundary
docs/known-gaps.md Effect symmetry note
flake.nix developer-shell ripgrep
```

## 10. Metrics and acceptance gates

### 10.1 Reproducible metrics

Use the same frozen commands from `IMPL_PLAN_RFC_FOLLOWUPS.md` for members, edges, source LOC,
external-test LOC, and mechanical public declarations. Retain the sorted edge/public lists in the
handoff, not the repository.

For each repair commit record:

```bash
git diff --numstat <parent>..<commit> -- \
  ':(glob)crates/**/src/**/*.rs' \
  ':(glob)bin/**/src/**/*.rs'
```

For the cumulative repair record, always use the reviewed implementation base and exclude exactly
this requested plan document:

```bash
repair_base=af4574f7babdaa9ea79d9886c313216d236f0bb2
git diff --numstat "${repair_base}"..HEAD -- . \
  ':(exclude)FIXES_IMPL_PLAN_RFC_FOLLOWUPS.md'
wc -l crates/kernel/program/src/authoring.rs
```

Also report the unfiltered repository delta including this plan. The filtered number is explicitly
“implementation LOC”, not total repository churn. No other documentation, fixture, test, or source
path may be excluded.

Recompute the selected domain ranges after formatting and use the immutable final SHA/range-ledger
recipe from the completed implementation plan. Count imports/rustdoc/private glue used only by the
selected blocks; do not relocate code to escape the metric.

### 10.2 Hard complexity gates

The final tree must satisfy:

```text
workspace members                         18
normal internal MFM dependency edges      45
mechanical public declarations          <=197
new public authoring items/methods          0
new packages / lockfile / Nix edges          0
new internal MFM dependency edges            0
App Tokio dependency role          dev -> normal
authoring.rs                           <=1,142 (target <=1,102)
domain selected-authoring delta          <=0
combined selected-authoring delta        <=+40 (target <=0)
implementation tracked-line delta        <=-800
manifest + scanner source lines deleted   >=1,333
cutover manifest/scanner lines              0
negative-scan tasks/edges                   0
CI leaf tasks                               9
```

The `+40` selected-authoring ceiling is not a budget to spend. Any positive line must be assigned
to the identity memo or a semantic lowering invariant and independently justified. Tests, rustdoc, or error
handling cannot be deleted merely to hit a number.

### 10.3 Identity gates

Assign identity evidence to the commit that can execute and owns it:

- Commit 1 retains the current representative Program length/ref assertion, declaration topology,
  518-at-64/65-rejected capacity, and all existing Program authoring goldens.
- Commit 2 captures and asserts complete representative Program/C0 bytes and refs, every State/
  capability/binding ref, and the three public EVM/Portfolio result fixtures.
- Commit 3 owns both exact retained Program-wire fixtures and decoder hostility, and reruns the
  Commit-1/2 owners through its focused packages.
- Commit 4 has no semantic identity change; model/flake checks own its task graph. The final
  cumulative CI and architect recheck the complete Commit-1/2/3 evidence on the exact candidate.

Any byte/ref drift at its owner gate is `BLOCK`; this plan contains no persisted cutover.

### 10.4 Performance gate

The exact 64-source test uses the three fresh same-host warmed medians from Section 6.5. The
candidate must be at most `10.00s`, at most 2x the `f6b62e35` median, and at most 0.1x the
`af4574f7` median. Deterministic top-level cache-miss counts remain the ongoing regression;
wall-clock evidence is a handoff gate only.

## 11. Verification sequence

Use focused commands while developing. Do not run broad gates merely because a commit is about to
be created.

Commit 1:

```text
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test -p mfm-portfolio --test planning_contract \
  representative_program_identity_is_stable -- --exact
```

Commit 2:

```text
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task capacity-app
```

Commit 3:

```text
nix develop -c cargo test -p mfm-program -p mfm-runtime --all-targets
nix develop -c cargo test -p mfm-portfolio -p mfm-app --all-targets
```

Commit 4:

```text
nix run .#model-check
nix flake check --no-build
```

After all focused failures are resolved, run:

```text
git diff --check
```

After the cumulative architect approves that exact tree, run exactly one final:

```text
nix run .#ci
```

The final CI must report nine successful leaves. Do not run `negative-scan`; it no longer exists.
Do not immediately precede final CI with redundant `.#check`, `.#test`, `.#test-db`, or broad
capacity gates on the same tree.

## 12. Completion checklist

The repair is complete only when all are true:

1. the four exact commits in Section 4 exist in order with lower-case subjects;
2. every chunk and all four commit gates have non-author architect approval;
3. a cumulative non-author architect approves the exact final candidate;
4. each directly authoring-requested value, State, or capability `TypeId`/category has at most one
   successful top-level memo miss per expansion; nested descriptor work and `Program::new`'s
   independent framework-`Never` validation remain outside that claim;
5. cache lifetime is exactly one `expand_program` and contains no semantic/callback result;
6. all callback/hook/binding occurrence counts remain exact;
7. `CollectEvmBalances` stores one binding and not the whole target;
8. all six EVM policies use `Setup = ContentRef` explicitly;
9. `start_portfolio` offloads only owned pure planning through immediately awaited
   `spawn_blocking`, preserves error classification, bounds targets at 64 before cloning, and
   Runtime entry remains async-side;
10. exact Program/C0 bytes, refs, topology, capacities, and public fixtures are unchanged;
11. the final performance envelope passes;
12. redundant entry/input/root scans and `Box<DraftState>` are absent;
13. Match retry and handler failure cannot partially mutate their semantic parent, fixed limits are
    checked before logical commit, and no partial typed allocator-recovery policy survives;
14. public decoder hostility lives in external retained-wire tests;
15. compile-fail tests prove constructor/method opacity and callback-reference non-escape;
16. exact representative Program and C0 fixture bytes are checked;
17. Runtime production is unchanged and the missing payload-codec association case rejects;
18. the manifest, scanner, empty scripts directory, task, CI edge, and task-only tools are absent;
19. no replacement scanner/tombstone manifest/source parser exists;
20. current workflow docs and current RFC text no longer require negative-scan;
21. trusted direct callback composition is documented honestly as a review rule;
22. workspace members/edges/public surface remain within Section 10;
23. implementation tracked lines (excluding exactly this plan) decrease by at least 800 and all
    1,333 manifest/scanner lines are deleted;
24. focused checks, model-check, flake evaluation, diff check, and one final nine-leaf CI pass; and
25. the worktree is clean.

## 13. Final handoff record

The engineer reports:

1. four commit SHAs/subjects and clean status;
2. every chunk/commit/cumulative architect decision and resolved `BLOCK`;
3. identity-cache call-count evidence;
4. before/after maximum-plan timings and median;
5. exact Program/C0/wire/result identity evidence;
6. public/API/dependency/member/edge metrics;
7. `authoring.rs`, selected-authoring, production churn, test churn, and cumulative tracked-line
   deltas;
8. exact manifest/scanner/task/tool deletion evidence;
9. focused/model/flake/diff/final-CI commands and results; and
10. the three Material uncertainties above, with performance resolved by measurement,
    trusted-callback ownership resolved by the approved RFC amendment, and the target bound
    resolved by caller inventory plus its 64/65 regression.

Attach this evidence to the PR/task handoff. Do not add a permanent progress log, approval
database, benchmark artifact, generated metric ledger, or compatibility manifest to the
repository.
