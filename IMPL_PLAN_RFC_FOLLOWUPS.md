# Implementation plan: semantic contracts, Operation expansion, and capability injection

Status: approved implementation handoff for
[`RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`](RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md)

Audience: engineer-agents and non-author architect reviewers implementing the complete source
authoring cutover

---

## 0. Authority, base, and stopping rule

The RFC is the approved architecture. It owns Operation semantics, capability-injection semantics,
the unchanged Program v2 wire, and the accepted material uncertainties. This plan fixes the exact
Rust API spelling, private lowering strategy, package/file ownership, deletions, test migration,
review gates, evidence, and two commit boundaries.

The reviewed implementation base is commit
`599c5a5bf6008b04bb39eaa81c010fc842ffda1c`. The approved RFC input at handoff has SHA-256:

```text
efda5b1227d241142b13f2823ed7cf1bcb9a5d9332c8f66bf8494e2da87a73eb
```

If either base changes before implementation, first re-run the inventories and exact Program
goldens in Section 12. Do not silently carry line counts or constructor-use assumptions across a
different tree.

The already implemented Runtime/Journal/Store cutover and TT1 fixes remain authoritative. This
work must not reopen Program v2, Runtime association/fold, Journal framing, Store persistence,
PostgreSQL, App production APIs, or Effect/submission retirement.

Breaking the current source-authoring API is intentional. Do not add aliases, deprecated exports,
feature-gated old constructors, a `ProgramAuthor`, a second builder, dual authoring, or a fallback.
Git history is the compatibility record.

If a chunk cannot implement an RFC invariant with the five public concepts and callable inventory
in Section 2, stop that chunk. Record the smallest concrete counterexample, ask a dedicated
architect for one replacement design, amend the RFC and this plan, and obtain approval. Do not
solve a private lowering difficulty by adding public labels, routes, an AST, a new error family, or
Runtime behavior.

### Material uncertainties

These are the four accepted RFC uncertainties; implementation adds no new material choice.

1. **Future durable Effect shape**
   - **Choice:** implement only Read occurrence injection and a Pure-only `InjectionWriter` now.
     Add no Effect placeholder and no nested Read emission.
   - **Why uncertain:** nonce authority, outbox/command durability, signer custody, provider
     ambiguity, and exact Effect failure contracts are not designed.
   - **Consequence if wrong:** the future Effect RFC may extend the writer or one structured
     failure rule, but it must continue through this sole compiler rather than add another one.
   - **Resolution:** prototype the complete durable transaction Operation before any Effect ABI,
     Program wire, Runtime, or Journal change is approved.

2. **First production recovering handler**
   - **Choice:** implement the RFC's exact forward recovery-to-`J` rule, while the current
     Portfolio use is terminal-to-`O` and the recovery proof is synthetic.
   - **Why uncertain:** no production `EvmBalanceFailure` currently carries a complete recovery
     context.
   - **Consequence if wrong:** a future recovery may require one deliberate additional construct;
     it must not be approximated with ambient context, backward edges, or raw routes.
   - **Resolution:** the first production recovery must supply a complete domain-owned payload and
     pass hot/cold Match/rejoin proof before stabilizing more API.

3. **Selector descriptor breadth**
   - **Choice:** author only the external/adjacent, single embedded nominal payload selector
     shapes already accepted by Runtime, including the one-argument `mfm/generic-value` form.
   - **Why uncertain:** future closed sums may use another persisted descriptor shape.
   - **Consequence if wrong:** authoring support expands locally; it does not justify public raw
     Match routes or moving Runtime association authority.
   - **Resolution:** freeze external, adjacent, generic, and hostile unsupported-shape tests in the
     Program and Runtime owners.

4. **Trusted open callbacks**
   - **Choice:** treat `Operation::expand` and injection hooks like trusted State code. Repository
     production code composes only through kernel methods; the scanner rejects direct method-call
     bypasses.
   - **Why uncertain:** Rust cannot mechanically prevent a downstream open-trait implementation
     from directly calling another public trait method.
   - **Consequence if wrong:** direct calls bypass child/policy ownership and callback-depth
     accounting, although final Program validation still guards persisted graph structure.
   - **Resolution:** keep the trust contract, rustdoc, scanner, and tests explicit. If downstream
     sandboxing becomes a requirement, redesign the trait before implementation rather than add a
     partial runtime check.

## 1. Completion rule and non-goals

The work is complete only when:

1. exactly the two lower-case commits in Section 4 exist, in order, and each is independently
   coherent;
2. every chunk has a recorded non-author architect `APPROVE` under Section 3;
3. all RFC acceptance criteria pass;
4. all stable IDs, associated contracts, Program schema, representative Program bytes/content
   reference, result fixtures, and hot/cold behavior are unchanged;
5. the semantic numeric families, raw domain planners, index/count forecasts, public raw Program
   constructors, and every prohibited alternate API in Section 7 are absent;
6. `expand_program` is the sole source-authoring ingress and `Program::decode_canonical` the sole
   retained-byte ingress;
7. Runtime, Journal, Store, PostgreSQL, App production Rust, CLI/REST, signing, and keystore have no
   production change;
8. workspace members remain 18 and normal internal MFM dependency edges remain 45;
9. the final public authoring surface is exactly the five concepts, one root function, and eleven
   callable trait/compiler/writer items in Section 2;
10. the measured combined Program/EVM/Portfolio authoring implementation meets the deletion and
    LOC gate in Section 12.3; and
11. focused checks and exactly one final composed CI run are green.

Do not implement any of the following in this cutover:

- Effect, nonce, signing, broadcast, submission, outbox, mutation, acknowledgement, or migration;
- public labels, addresses, successors, routes, `_to` methods, raw `select`, or failure policies;
- a public/private persisted Operation AST, recipe/node enum, HList, dynamic registry, `Any`,
  boxed Operation, or stored callback;
- `ProgramAuthor` or `OperationExpansion -> ProgramAuthor -> Program`;
- conditional/no-op Reads or `NotApplicable` evidence;
- `InjectionWriter::read`, `InjectionWriter::operation`, Match/handler methods on the writer, or an
  Effect placeholder;
- a blanket capability-injection implementation;
- a new Program/Operation error type or variant;
- generic Application dispatch, functional transports, automatic progression, scheduling, facts,
  replay, or configuration publication; or
- a test-only public constructor/feature that bypasses the final API.

## 2. Frozen final public API and ownership

### 2.1 Exact `mfm-program` surface

Add these six crate-root exports and no authoring aliases:

```rust
pub trait Operation: Sized {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: MfmValue;

    fn expand(
        &self,
        expansion: &mut OperationExpansion<
            Self::Input,
            Self::Output,
            Self::Failure,
        >,
    ) -> Result<()>;
}

pub fn expand_program<O: Operation>(
    entry_point_id: EntryPointId,
    root: &O,
) -> Result<Program>;

pub struct OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    // private
}

impl<I, O, F> OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    pub fn pure<S: PureState>(&mut self) -> Result<()>;

    pub fn read<S, C>(
        &mut self,
        setup: &<C as CapabilityInjection<S>>::Setup,
    ) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract + CapabilityInjection<S>;

    pub fn operation<Op: Operation>(&mut self, child: &Op) -> Result<()>;

    pub fn match_join<T, J>(
        &mut self,
        define: impl FnOnce(&mut MatchJoin<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        T: MfmValue,
        J: MfmValue;

    pub fn with_failure_handler<E, J>(
        &mut self,
        protected: impl FnOnce(&mut Self) -> Result<()>,
        handler: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()>
    where
        E: MfmValue,
        J: MfmValue;
}

pub struct MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    // private
}

impl<I, O, F> MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    pub fn arm<P>(
        &mut self,
        tag: StableId,
        branch: impl FnOnce(&mut OperationExpansion<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        P: MfmValue;
}

pub trait CapabilityInjection<S>
where
    S: State,
{
    type Setup;
    type ExpandedInput: MfmValue;
    type ExpandedOutput: MfmValue;

    fn original_binding_ref(setup: &Self::Setup) -> Result<ContentRef>;

    fn write_before(
        _setup: &Self::Setup,
        _writer: &mut InjectionWriter,
    ) -> Result<()> {
        Ok(())
    }

    fn write_after(
        _setup: &Self::Setup,
        _writer: &mut InjectionWriter,
    ) -> Result<()> {
        Ok(())
    }
}

pub struct InjectionWriter {
    // private
}

impl InjectionWriter {
    pub fn pure<S: PureState>(&mut self) -> Result<()>;
}
```

The exact budget is:

- five logical public concepts;
- one root `expand_program` function;
- `Operation::expand`;
- three `CapabilityInjection` functions;
- five `OperationExpansion` methods;
- `MatchJoin::arm`; and
- `InjectionWriter::pure`.

That is eleven callable trait/compiler/writer items plus the root function. Exceeding it is an
architect `BLOCK`. Do not add constructors, getters, aliases, convenience overloads, route types,
or a `return_success` method.

`OperationExpansion` and `MatchJoin` use private markers such as
`PhantomData<fn(I) -> (O, F)>` if their dynamic private fields do not otherwise represent the
generic contracts. `MatchJoin` and `InjectionWriter` own their scratch and expose no lifetime.
None of `OperationExpansion`, `MatchJoin`, or `InjectionWriter` has a public constructor, `Clone`,
or `Default`. All new public items have complete rustdoc and the crate README contains one runnable
root-authoring example.

### 2.2 Constructor visibility after cutover

Keep declaration/Program types and all read-only accessors public. Keep
`Program::decode_canonical` public. Make exactly these source constructors `pub(crate)`:

```text
Program::new
StateDeclaration::new
Execution::pure
Execution::read
MatchDeclaration::new
MatchVariant::new
```

`StateDeclaration::new` is currently infallible despite returning `Result`. The smallest private
cleanup is to return `Self` and wrap it only in `RawState::try_checked`; do not replace it with a
new builder.

### 2.3 Dependency and responsibility freeze

No Cargo dependency changes are required. `mfm-program` already owns every dependency needed by
the DSL. EVM and Portfolio already depend on Program. Final workspace members and normal internal
edges remain exactly 18 and 45.

| Owner | Final responsibility in this cut |
| --- | --- |
| `mfm-program` | Public typed DSL, one private flat draft, structured lowering, final Program construction, raw decode/validation |
| `mfm-evm` | Semantic capability/State contracts, six exact injection policies, configured balance child Operation |
| `mfm-portfolio` | Semantic Portfolio States, checked root Operation, request/C0 authoring |
| `mfm-evm-live` | Registration of the three semantic capability types only |
| `mfm-app` tests | Trusted registration and unchanged end-to-end behavior proof |
| Runtime | Unchanged production association/fold; test fixtures consume the new source API or retained-byte ingress |

No other production crate gains Operation or injection ownership.

## 3. Mandatory architect review protocol

Every chunk in Sections 5 and 6 requires one named architect reviewer who did not implement that
chunk. A reviewer may cover multiple chunks only with a separate decision for each. No chunk may
be called done, and no later dependent chunk may be integrated, while its latest decision is
`BLOCK`.

Commit 2 chunks accumulate in one uncommitted worktree because constructor privacy, source-author
migration, and test migration are inseparable. They are not extra commits. A temporary compiler
seam or dual path may exist only inside that worktree and must be called out to the reviewer; it
must never appear at either submitted commit boundary.

For every chunk, give the reviewer:

1. this plan, the RFC, `AGENTS.md`, `docs/design.md`, `docs/architecture.md`, and
   `docs/code-quality.md`;
2. the path-scoped diff and the cumulative dependency diff;
3. the exact old symbols/files to delete and `rg` evidence of remaining owners;
4. focused compile/test output, or a precise integration-seam explanation;
5. production Rust added/deleted, public items/methods changed, dependency edges changed, and the
   affected authoring change sites; and
6. every earlier block plus its resolution.

The architect reviews for:

- correctness of exact contracts, graph topology, atomicity, hostile input, and Program-byte
  identity;
- Rust feasibility, generic/lifetime ownership, private construction authority, and redacted
  errors;
- dexterity: one State/Operation/injection change remains in its semantic owner and does not touch
  Runtime/Journal/Store/App production;
- simplicity: minimum concepts, methods, private types, allocations, passes, and LOC;
- one invariant owner, one flat draft, one final Program vector, and no duplicate validator;
- deletion completeness without aliases, fallbacks, wrappers, or speculative extension points;
  and
- tests at the owning boundary rather than duplicated broad fixtures.

Every review applies to an exact cumulative candidate tree. Start the implementation from a clean
index, stage the complete cumulative candidate that the reviewer sees, and record its
`git write-tree` object ID plus the unchanged parent commit. The tree object is evidence, not an
extra commit. If an approved chunk's paths change later, that chunk's reviewer must review the new
tree or explicitly carry the earlier decision forward after inspecting the delta.

Every review uses this decision record:

```text
Review scope: C1-A | ... | C2-H | COMMIT-1 | COMMIT-2 | CUMULATIVE
Parent commit: <sha>
Candidate tree: <git write-tree object id>
Reviewer: <non-author>
Decision: APPROVE | BLOCK

Correctness findings:
- ...

Dexterity findings:
- current files/change sites for one State, Operation, and injection pairing
- whether ownership remains local

Simplicity and LOC findings:
- public concepts/methods and private helpers added/deleted
- production Rust +/-, dependency edges, allocations/passes
- smaller complete implementation considered

Deletion evidence:
- exact symbols/paths/uses absent or deferred to the named inseparable chunk

Verification evidence reviewed:
- exact commands and results

Material uncertainties:
none | exact uncertainty, consequence, and resolution
```

`BLOCK` is mandatory for a correctness defect, public budget breach, alternate builder/final form,
whole-parent cloning, duplicate validation owner, stale raw constructor, missing deletion, positive
LOC hidden by relocation, unowned dependency edge, or any new material uncertainty. Passing tests
does not override a block.

Review records belong in the PR/task handoff, not a permanent approval log in the repository.

## 4. Exact commit graph

There are exactly two implementation commits:

```text
1. name surviving evm and portfolio contracts
   |
2. centralize operation expansion and capability injection
```

Commit 1 changes source names only and must preserve the still-manual Program authoring. Commit 2
adds the sole DSL and removes every raw authoring path in one coherent cutover. Do not split public
API addition from migration/deletion, and do not squash the semantic-name cutover into Commit 2.

## 5. Commit 1 — `name surviving evm and portfolio contracts`

### 5.0 Commit result

At this boundary, the graph is still authored by the existing EVM/Portfolio raw declaration code,
but all supported Rust contracts use semantic names. Stable IDs, generic continuation types,
execution class, bindings, declarations, Program bytes, fixtures, dependencies, and Runtime/live
behavior are byte-for-byte or behaviorally unchanged.

This commit has three reviewed chunks.

### 5.1 Chunk C1-A — semantic EVM capabilities and States

Primary paths:

- `crates/domains/evm/src/lib.rs`;
- its existing inline `#[cfg(test)]` module;
- `crates/live/evm/src/lib.rs`;
- `crates/app/tests/portfolio_runtime.rs`, only for its eight EVM State registrations, including
  all six Read capability type arguments.

Replace the generic capability family:

```rust
pub struct EvmCapability<const KIND: u8>;
```

with exactly these documented zero-sized types:

```rust
pub struct EvmChainIdentityRead;
pub struct EvmAnchorRead;
pub struct EvmBalanceRead;
```

Retain `impl_read_capability!` only as a small implementation deduplicator. Its invocations use the
semantic type directly and freeze these exact IDs and families:

| New capability | Stable ID | Family |
| --- | --- | --- |
| `EvmChainIdentityRead` | `mfm.evm.capability.read-chain-identity@1` | `ChainIdentity` |
| `EvmAnchorRead` | `mfm.evm.capability.read-anchor@1` | `LatestAnchor` |
| `EvmBalanceRead` | `mfm.evm.capability.read-balance@1` | `Balance` |

All three keep `Intent = EvmReadIntent`, `Evidence = EvmReadEvidence`, and the existing
`bind_evidence` implementation.

Replace:

```rust
pub struct EvmState<const FAMILY: u8, const STAGE: u8, K: MfmValueTrait>(
    PhantomData<fn() -> K>,
);
```

with eight documented, non-constructible semantic ZSTs:

```rust
pub struct CheckChainIdentity<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ReadInitialAnchor<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct SelectBalanceAsset<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ReadNativeBalance<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ReadTokenDecimals<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ReadTokenBalance<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ConfirmBalanceAnchor<K: MfmValueTrait>(PhantomData<fn() -> K>);
pub struct ConsolidateBalanceCollection<K: MfmValueTrait>(PhantomData<fn() -> K>);
```

The `PhantomData` fields stay private. Do not add constructors; these types are static contracts,
not runtime values. Replace stage-based macro inputs with semantic type inputs. Remove the unused
`$failure_stage` macro argument rather than carrying it into the named form.

Freeze this complete mapping:

| Semantic State | Input | Output | Failure | Execution/capability | Stable ID |
| --- | --- | --- | --- | --- | --- |
| `CheckChainIdentity<K>` | `EvmBalanceContext<K>` | same | `EvmBalanceFailure` | Read / `EvmChainIdentityRead` | `mfm.evm.state.check-chain-identity@1` |
| `ReadInitialAnchor<K>` | context | context | failure | Read / `EvmAnchorRead` | `mfm.evm.state.read-initial-anchor@1` |
| `SelectBalanceAsset<K>` | context | `EvmBalanceAsset<K>` | failure | Pure | `mfm.evm.state.select-asset@1` |
| `ReadNativeBalance<K>` | context | context | failure | Read / `EvmBalanceRead` | `mfm.evm.state.read-native-balance@1` |
| `ReadTokenDecimals<K>` | context | context | failure | Read / `EvmBalanceRead` | `mfm.evm.state.read-token-decimals@1` |
| `ReadTokenBalance<K>` | context | context | failure | Read / `EvmBalanceRead` | `mfm.evm.state.read-token-balance@1` |
| `ConfirmBalanceAnchor<K>` | context | context | failure | Read / `EvmAnchorRead` | `mfm.evm.state.confirm-balance-anchor@1` |
| `ConsolidateBalanceCollection<K>` | context | `EvmBalanceCollectionCompletion<K>` | failure | Pure | `mfm.evm.state.consolidate-balance-collection@1` |

Here `context` means the exact `EvmBalanceContext<K>` and `failure` means the exact
`EvmBalanceFailure`; it is only table compression. The implementation spells the complete types.

Update the existing behavior implementations and the still-temporary raw `append_balance_fragment`
to use these names. Do not change its arithmetic in this commit. Migrate both numeric pair
occurrences in `ordinary_read_interpreter_maps_every_failure_evidence_variant` and the numeric
filler occurrence in `balance_fragment_derives_every_internal_index_from_prefilled_program_order`.
Update live registration to the three semantic capability types in this same chunk. Migrate the
App test's EVM registrations without changing any Portfolio State name, assembly order, or
behavior. The Portfolio planner calls the retained public fragment helper and contains no numeric
EVM type occurrence; prove its emitted graph remains unchanged rather than editing it in C1-A.
This closes every supported cross-crate EVM type consumer. Do not introduce aliases such as `type
EvmState0<K> = ...`.

Required C1-A tests/evidence:

- exact capability IDs and State IDs for all eleven types;
- exact State Input/Output/Failure and Read capability pairings;
- unchanged intent/evidence validation;
- unchanged live adapter registration/provider behavior;
- unchanged still-manual Portfolio graph and explicit App assembly after the EVM-only name
  migration;
- no `EvmCapability<...>` or `EvmState<...>` use remains in supported production Rust; and
- the path diff changes names/macro axes, not behavior bodies or persisted strings.

Architect C1-A specifically compares every old generic instantiation to the new implementation and
traces the live and App EVM type consumers while proving the still-manual Portfolio graph emitted
through the helper is unchanged. It `BLOCK`s an ID, contract, execution-class, behavior,
visibility, consumer, or macro-generalization drift.

### 5.2 Chunk C1-B — semantic Portfolio States and consumers

Primary paths:

- `crates/domains/portfolio/src/lib.rs`;
- `crates/domains/portfolio/tests/planning_contract.rs`;
- `crates/app/tests/portfolio_runtime.rs`.

Replace:

```rust
pub struct PortfolioState<const STAGE: u8>;
```

with exactly:

```rust
pub struct InitializePortfolio;
pub struct EnterPortfolioCollection;
pub struct ResumePortfolioCollection;
pub struct MapEvmBalanceFailure;
pub struct ConsolidatePortfolio;
```

Freeze the existing contracts and IDs:

| Semantic State | Input | Output | Failure | Stable ID |
| --- | --- | --- | --- | --- |
| `InitializePortfolio` | `PortfolioSnapshotInput` | `PortfolioContinuation` | `PortfolioSnapshotFailure` | `mfm.portfolio.state.initialize@1` |
| `EnterPortfolioCollection` | `PortfolioContinuation` | `EvmBalanceContext<PortfolioContinuation>` | failure | `mfm.portfolio.state.enter-collection@1` |
| `ResumePortfolioCollection` | `EvmBalanceCollectionCompletion<PortfolioContinuation>` | `PortfolioContinuation` | failure | `mfm.portfolio.state.resume-collection@1` |
| `MapEvmBalanceFailure` | `EvmBalanceFailure` | `PortfolioSnapshotOutput` | failure | `mfm.portfolio.state.map-evm-failure@1` |
| `ConsolidatePortfolio` | `PortfolioContinuation` | `PortfolioSnapshotOutput` | failure | `mfm.portfolio.state.consolidate@1` |

All five remain Pure. Remove the ordinal rustdoc and `$stage` macro input. A small macro may still
generate the repeated `State`/`PureState` impl shape only when each invocation names the semantic
type directly.

Update the still-temporary raw Portfolio author from the five numeric Portfolio State occurrences
to the five semantic Portfolio types without changing any index. Update the App integration
assembly's five Portfolio registrations; its eight EVM State registrations and all six Read
capability type arguments were already migrated in C1-A. The final assembly names all thirteen
States explicitly. Do not add a registration bundle or App production helper merely to shorten a
test.

Required C1-B identity proof:

```text
Program canonical bytes: 36,560
Program schema:
  schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c
Program digest:
  content:sha256-v1:59c503d1d8faa1a7afc33d9bb0bcad42d053e7cecb8571023bbc0cef42ab4beb
```

Also preserve:

- one collection/64 sources = 518 declarations;
- 65 sources is rejected;
- these three retained public result fixtures byte-for-byte:
  - `docs/contracts/evm-portfolio/evm-balance-collection.json`;
  - `docs/contracts/evm-portfolio/portfolio-snapshot.json`; and
  - `docs/contracts/evm-portfolio/portfolio-snapshot-failure.json`;
- the live adapter registration/provider behavior already migrated and proved in C1-A; and
- hot/cold App Portfolio behavior.

The final trusted composition boundary is explicit: the App test registers thirteen semantic
States (eight EVM and five Portfolio), while `mfm-evm-live` registers exactly three adapters for
the three semantic Read capability types.

Architect C1-B traces all sixteen semantic State/capability registrations and `BLOCK`s a helper
that hides the explicit composition boundary, an App production change, or any Program/C0/fixture
drift.

### 5.3 Chunk C1-C — current docs, numeric absence, and commit evidence

Primary paths:

- `crates/domains/evm/README.md`;
- `crates/domains/portfolio/README.md`;
- `crates/live/evm/README.md` when it names registration contracts;
- directly affected rustdoc;
- `RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md`; and
- `scripts/check-cutover-manifest.sh` only for the necessary new canaries.

Rewrite current docs from numeric families/stages to semantic names. Do not teach the Operation API
yet. The Program construction remains manual at this boundary.

Append owner-scoped production absence rules for:

```text
\bEvmCapability\s*<
\bEvmState\s*<
\bPortfolioState\s*<
```

Do not globally ban numeric generics or the words capability/state. Add a canary for each new rule,
map every added ledger entry to a machine rule, and refresh ledger/ruleset/coverage/inventory
fingerprints using the scanner's existing extraction logic. The script's deliberate canary text is
not a production match and must not be removed to make a scan pass.

Required searches, with narrow exceptions only for top-level design records and deliberate hostile
test fixtures:

```bash
rg -n 'EvmCapability\s*<|EvmState\s*<|PortfolioState\s*<' \
  crates bin docs README.md AGENTS.md
rg -n 'family and stage|stage ordinal|0 initialize|kind [267]' \
  crates/domains/evm crates/domains/portfolio crates/live/evm docs
```

Architect C1-C checks that the scanner scope remains supported-current and category-specific, that
no broad exclusion is added, and that all docs describe the independently coherent Commit-1 tree.

### 5.4 Commit 1 integration gate and definition of done

Run:

```bash
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task negative-scan
git diff --check
```

Do not run broad workspace gates merely because the commit is about to be made. Commit 1 is ready
only when:

- C1-A, C1-B, and C1-C are `APPROVE`;
- one non-author domain-contract architect has inspected the exact integrated Commit-1 candidate
  and recorded a separate `COMMIT-1 APPROVE`, covering every old/new association, stable identity,
  public-surface tradeoff, owner-local change site, deletion, and focused verification;
- all numeric generic families/uses/aliases are absent from supported production/current docs;
- exactly 3 capabilities and 13 State semantic names replace the 3 generic public families;
- IDs, contracts, bytes, fixtures, behavior, members, dependency edges, and production topology
  are unchanged;
- no Operation DSL or compatibility alias landed early; and
- the commit subject is exactly `name surviving evm and portfolio contracts`.

## 6. Commit 2 — `centralize operation expansion and capability injection`

### 6.0 How to execute this inseparable cutover

Use these cumulative uncommitted chunks:

```text
C2-A  one private draft, root/State/child lexical lowering
C2-B  capability-owned Read injection
C2-C  structured Match joins
C2-D  scoped failure handling
C2-E  EVM configured Operation and six policies
C2-F  Portfolio root migration and both domain raw-author deletions
C2-G  raw constructor privacy and Program/Runtime test migration
C2-H  docs, scanner, complexity evidence, cumulative gate
```

C2-E and C2-F form one compile seam: EVM cannot delete `append_balance_fragment` until Portfolio
stops calling it. Implement both in the same compile-green window, then give their path-scoped
diffs to separate reviewers. No commit or review handoff may misrepresent the temporary duplicate
path as final design.

The final public API is added and all raw source authors are removed in this one commit. There is
no intermediate compatibility release.

### 6.1 Chunk C2-A — one private draft and lexical Operation lowering

Primary paths:

- `crates/kernel/program/src/lib.rs`;
- new private `crates/kernel/program/src/authoring.rs`;
- new `crates/kernel/program/tests/authoring.rs`; and
- `crates/kernel/program/README.md` for the new root example, completed in C2-H.

Keep Program v2 wire/decode/validation in `lib.rs`. Declare `mod authoring;` privately and reexport
only the six approved public items from the crate root. Do not expose the module or its private
draft types.

#### C2-A.1 Smallest private representation

Use one flat representation for root and every composite scratch segment. The spelling below is
the target unless the chunk architect approves a strictly smaller equivalent with the same one-pass
semantics:

```rust
const MAX_AUTHORING_CALLBACK_DEPTH: u8 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DraftId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DraftRoute {
    Open,
    Target(DraftId),
    Terminal,
}

struct DraftState {
    state_implementation_ref: ContentRef,
    input_contract_ref: ContentRef,
    output_contract_ref: ContentRef,
    failure_contract_ref: ContentRef,
    execution: Execution,
    next: DraftRoute,
    failure_next: DraftRoute,
}

struct DraftMatchVariant {
    tag: StableId,
    entry: DraftId,
}

struct DraftMatch {
    selector_contract_ref: ContentRef,
    variants: Vec<DraftMatchVariant>,
}

enum DraftDeclaration {
    State(DraftState),
    Match(DraftMatch),
}

struct Frontier {
    contract_ref: ContentRef,
    tails: Vec<DraftId>, // State success slots whose next route is Open
}

struct ExpansionDraft {
    declarations: Vec<DraftDeclaration>,
    state_count: usize,
    entry: Option<DraftId>,
    open_success: Option<Frontier>,
    scope_success: Vec<DraftId>,
    open_failures: Vec<DraftId>,
}
```

An empty initialized scratch has `entry = None` and an open frontier at its exact input with no
tails. `open_success = None` means that path is already closed. `scope_success` contains State
success slots deliberately closed at the current Operation's `O`. `open_failures` contains only
inhabited unresolved State failure slots; inspect each `DraftState.failure_contract_ref` rather
than add handler/failure wrapper objects.

`OperationExpansion` privately carries:

- the exact scope input/output/failure refs;
- one owned `ExpansionDraft`;
- the failure contract refs currently admitted by enclosing structured handlers;
- the current kernel-entered callback depth; and
- a non-owning `I/O/F` marker when needed.

Do not add handler IDs/targets, a label table, rollback log, State seed, branch outcome, scope-exit
enum, per-State scratch draft, parent clone, route arena, nested Operation tree, or second final
declaration form. Composite constructs reuse `ExpansionDraft` as short-lived scratch and rebase it
once.

#### C2-A.2 Private draft primitives

Implement only these responsibilities, sharing them across later chunks:

1. `append_state_core`
   - derive every ref and the complete `Execution` before mutation;
   - require an open frontier whose contract equals the State input;
   - reject `Never` input/output;
   - validate the inhabited failure contract is the scope failure or an admitted handler contract;
   - preflight declaration and State ceilings;
   - require each prior tail still has `next = Open`, patch those tails to the new `DraftId`, and
     append exactly one State;
   - use `failure_next = Terminal` for `Never`, otherwise `Open` plus one `open_failures` entry; and
   - replace the frontier with the State's output and new success tail.
2. `rebase`
   - checked-add one offset to internal `Target` routes, Match entries, `entry`, frontier tails,
     `scope_success`, and `open_failures`;
   - perform every allocation/count/u16 preflight before returning a rebased owned scratch.
3. connected merge
   - validate the parent's complete open frontier and scratch entry before mutation;
   - connect every parent tail to scratch entry, append the rebased scratch once, and install its
     cursor/exits; and
   - treat an empty scratch only according to the caller's explicit root/child rule.
4. unconnected merge
   - append independently authored arm/handler blocks without connecting the preceding frontier;
   - use only inside C2-C/C2-D composite scratch, never as public/general source authority.
5. scope transforms
   - seal exact open success tails into `scope_success`;
   - reopen a child-relative `scope_success` plus its ordinary exact output frontier as the
     parent's continuation;
   - partition `open_failures` by exact contract; and
   - reject unresolved routes before final lowering.

Build a composite entirely in owned scratch. Preflight its entry, counts, allocations, refs, and
rebase, then mutate the parent through one infallible merge. Do not clone the whole parent for
rollback; that makes repeated authoring quadratic.

#### C2-A.3 Callback depth and error taxonomy

Root expansion starts at depth 1. Before invoking a child Operation, Match definition/arm,
protected/handler callback, or injection binding/before/after hook, use:

```rust
fn nested_callback_depth(current: u8) -> Result<u8> {
    current
        .checked_add(1)
        .filter(|depth| *depth <= MAX_AUTHORING_CALLBACK_DEPTH)
        .ok_or(ProgramError::Capacity)
}
```

Reject the 65th active callback before invoking it. Pass depth by value into owned scratch; do not
add global/thread-local state, `Rc<Cell<_>>`, a semaphore, or recursion registry. Sequential hooks
at one nesting level each use the same parent-plus-one depth.

Callbacks are synchronous `FnOnce`, never stored, retried, caught, or run after source authoring.
Normalize callback-manufactured errors at an authoring boundary:

```rust
fn authoring_error(error: ProgramError) -> ProgramError {
    match error {
        ProgramError::Capacity => ProgramError::Capacity,
        ProgramError::Canonical | ProgramError::InvalidContract => {
            ProgramError::InvalidContract
        }
    }
}
```

Only retained decode/final canonical encoding owns `Canonical`. Identity, descriptor, continuity,
scope, unsupported-shape, callback, and unresolved-route failures are `InvalidContract`; fixed
depth/count/u16/byte limits are `Capacity`. Add no variant.

#### C2-A.4 Pure State, child Operation, and root algorithms

`pure::<S>` derives `S` implementation/Input/Output/Failure refs and `Execution::pure` before
calling the shared State append primitive. It accepts no runtime configuration.

`operation::<Op>` performs this exact transaction:

1. derive `Op::Input`, `Op::Output`, and `Op::Failure` refs;
2. require the parent frontier to be exact `Op::Input` and ensure `Op::Failure` is a legal parent
   failure;
3. enter the checked callback depth;
4. build a fresh child scope whose only root failure is `Op::Failure`; do not import parent
   handler contracts into child internals;
5. invoke `child.expand` exactly once;
6. reject an empty child;
7. require every ordinary open child success to be exact `Op::Output`;
8. require every unresolved child failure to be exact `Op::Failure`;
9. combine the child's ordinary output tails and child-relative `scope_success` tails into one
   exact `Op::Output` parent frontier;
10. carry the child failure slots outward as exact `Op::Failure`; and
11. merge once.

Child-relative successful exits reopen as the parent continuation. Only root finalization turns
root-relative success into `None`. This preserves early child success without accidentally ending
the parent Operation.

`expand_program`:

1. derives the root Input/Output/Failure refs and creates the root scratch;
2. invokes `root.expand` once at depth 1;
3. permits an empty root only when Input equals Output and Failure equals `Never`;
4. otherwise requires every open success to equal the root Output and every unresolved failure to
   equal the root Failure;
5. seals root open/scope success and root failure to terminal routes;
6. rejects any remaining `Open` route;
7. converts `Target(DraftId)` exactly once to checked forward `u16` indices;
8. creates one ordinary `Vec<Declaration>` in authored order; and
9. calls `Program::new` once, leaving final reachability/terminal/byte validation in Program.

Do not reimplement `validate_program`, canonical encoding, or content-ref derivation in
`authoring.rs`.

#### C2-A.5 Tests and architect gate

Add public DSL tests for:

- zero-State root identity and invalid empty roots;
- root `expand` invoked exactly once and never retried, including a returned error;
- one and repeated Pure States;
- exact lexical continuity and Never/inhabited failure classification;
- repeated configured child values;
- child output/failure reopening into a parent and root terminal sealing;
- child-relative early success;
- empty child, wrong contracts, capacity, and callback errors leaving the parent unchanged;
- direct/mutual recursion entered through `operation` accepting 64 and rejecting callback 65
  before invocation;
- child callback called once and never retried; and
- authored order retained exactly.

Architect C2-A traces a child through both success and failure into the final declaration vector.
It `BLOCK`s a second graph/final form, handler IDs, parent clone, rollback log, general route arena,
stored callback, global depth state, duplicate Program validator, or an extra public method/type.

### 6.2 Chunk C2-B — exact-pair capability-owned Read injection

Primary paths:

- `crates/kernel/program/src/authoring.rs`;
- `crates/kernel/program/tests/authoring.rs`; and
- test-only value/capability/State fixtures in that integration test.

Add the exact `CapabilityInjection`, `InjectionWriter`, and `OperationExpansion::read` signatures
from Section 2. Do not implement EVM pairings until C2-E.

#### C2-B.1 Writer and occurrence ownership

The writer owns only:

```rust
pub struct InjectionWriter {
    draft: ExpansionDraft,
    required_failure_contract_ref: ContentRef,
}
```

Fields remain private. `InjectionWriter::pure` delegates to the same State-core construction as
`OperationExpansion::pure`, but permits only `Never` or the exact designated original
`S::Failure`. The enclosing `read` call owns callback depth and has already proved that failure is
legal in its parent scope. The writer cannot see the designated Read seed/binding and cannot emit
another Read.

`CapabilityInjection::Setup` is sized. It is a checked, bounded, secret-free domain value. No
provider, client, signer, key, Store connection, Runtime, nonce authority, mutable global, or IO
handle enters the trait.

#### C2-B.2 `read` algorithm

Implement one private `expand_read_occurrence::<S, C>(setup)` used only by public `read`:

1. derive `C::ExpandedInput`, `C::ExpandedOutput`, `S::Input`, `S::Output`, and `S::Failure` refs;
2. require the parent frontier to be exact ExpandedInput and verify the original failure is legal
   in the parent;
3. create one empty writer scratch at ExpandedInput with the original failure as its private
   required failure;
4. enter one checked callback and invoke `<C as CapabilityInjection<S>>::write_before` once;
5. require the writer frontier to be exact `S::Input`;
6. enter one checked callback and invoke `original_binding_ref` once;
7. derive S's implementation ref, C's capability ref, and C::Intent/C::Evidence nominal refs;
8. append exactly one kernel-owned Read State using that binding;
9. enter one checked callback and invoke `write_after` once;
10. require the writer frontier to be exact ExpandedOutput;
11. require every injected Pure failure to be `Never` or exact `S::Failure`;
12. carry inhabited occurrence failure slots to the parent's structured failure/root owner; and
13. preflight/rebase/merge the whole before/original/after suffix once.

Success order is fixed:

```text
ExpandedInput -> before Pure* -> designated Read -> after Pure* -> ExpandedOutput
```

Any inhabited failure skips the unfinished suffix. There is no finally, retry, rollback,
compensation, Match, or recovery inside injection. A hook error, binding error, mismatch, or
capacity failure discards scratch and leaves the parent unchanged. Captured callback side effects
are forbidden but cannot be rolled back by the graph compiler.

#### C2-B.3 Tests and architect gate

Use explicit local test policies, never a blanket impl. Prove:

- identity injection emits one unchanged Read;
- synthetic multiple-Pure before/original/after lexical order and exact contracts, with the
  binding present only on the designated Read and no hidden binding on injected Pure States;
- two ordinary same-type Reads each independently invoke policy and emit their designated Read;
- hooks cannot emit/access/replace the Read or binding;
- exact setup used by hooks and binding derivation;
- two independently authored occurrences with identical checked setup produce identical Program
  canonical bytes and ContentRefs;
- Never/exact original failure accepted and a foreign failure rejected;
- before/original/after failure uses one enclosing failure route;
- hook/binding/contract/capacity errors are atomic;
- successful hooks once, entered failures no retry, later hook short-circuit;
- checked callback depth includes binding/before/after; and
- decode/Runtime/Journal/Store/adapter/provider paths never invoke hooks, covered finally by one
  Runtime integration counter in C2-G.

Architect C2-B traces one synthetic nonempty suffix into exact Program bytes and confirms one
kernel-owned original, one State append primitive, no blanket impl, no execution authority, and no
writer method beyond `pure`.

### 6.3 Chunk C2-C — structured Match joins

Primary paths:

- `crates/kernel/program/src/authoring.rs`;
- `crates/kernel/program/tests/authoring.rs`; and
- later Runtime hostile-association preservation in C2-G.

Add only `MatchJoin` and `OperationExpansion::match_join`/`MatchJoin::arm` from Section 2.

#### C2-C.1 Descriptor proof

Keep the source-authoring descriptor proof private in Program. Mirror, but do not import or move,
Runtime's association rule:

```rust
fn match_payload_descriptor(
    shape: &SchemaShape,
) -> Option<(&SchemaId, &SemanticTypeId, &SchemaShape)> {
    let payload = match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => &elements[0],
        other => other,
    };

    match payload {
        SchemaShape::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape,
        } => Some((schema_id, semantic_type_id, serialized_shape)),
        SchemaShape::Generic {
            constructor,
            arguments,
            serialized_shape,
        } if constructor == "mfm/generic-value" => {
            let [argument] = arguments.as_slice() else {
                return None;
            };
            Some((
                &argument.schema_id,
                &argument.semantic_type_id,
                serialized_shape,
            ))
        }
        _ => None,
    }
}
```

The actual code dereferences boxes correctly and may use a private checked descriptor struct.
Validate `T::schema_descriptor` is an external or adjacent Enum. Retain each exact tag, payload
schema ID, payload semantic ID, serialized shape, and a `seen` bit. `arm::<P>` compares all three
payload facts against P, not merely the content ref. Do not add a public `mfm-values` helper to
share these roughly local checks; Runtime remains the hostile retained-wire association owner.

#### C2-C.2 Match transaction

`match_join::<T, J>`:

1. requires the current frontier to be exact T;
2. validates the descriptor and arm ceiling before invoking the definition callback;
3. builds one composite scratch whose first declaration is Match;
4. invokes the definition callback once at checked depth;
5. rejects unknown/duplicate tag before invoking that arm callback;
6. creates each arm scratch at the exact P contract, inheriting the enclosing scope failure and
   admitted handler contract refs, then invokes its callback once;
7. requires the branch to be nonempty and State-first;
8. classifies an open exact J as join, an open distinct enclosing O as `scope_success`, and any
   other open contract as invalid;
9. retains nested already-closed scope success;
10. appends arm blocks unconnected in callback order and records their rebased first State;
11. after definition, rejects missing variants;
12. sorts only Match metadata by raw tag bytes through private `MatchDeclaration::new`;
13. aggregates all open J State tails into one frontier, or leaves the path closed when all arms
    are terminal; and
14. connects the parent's prior frontier to the Match and merges the complete construct once.

When `J == O`, check J first: an open result is join-only. A terminal nested construct remains
closed. Never connect one arm's J tails to the next physical arm. Match itself precedes every arm
body, and a variant targets the first State, including an injected-before State.

#### C2-C.3 Tests and architect gate

Test non-generic external, non-generic adjacent, one-argument-generic external, and
one-argument-generic adjacent selectors; also test heterogeneous payload types, State and
child-Operation arms, physical callback order versus sorted metadata, unequal arm lengths,
injected arm entry, mixed/all terminal, J==O, and nested closed success.

Hostile cases: non-enum, internal/unit/named/multi-field/unsupported generic, missing/unknown/
duplicate tag, wrong P/semantic/serialized shape, wrong J, empty arm, Match-first arm, arm/callback
failure, 256/257 arms, and parent snapshot atomicity.

Architect C2-C manually derives final indices for one unequal two-arm graph and `BLOCK`s public
labels/routes, raw selection, branch sorting, a second descriptor owner in Values, or a Runtime
production edit.

### 6.4 Chunk C2-D — scoped failure handling

Primary paths:

- `crates/kernel/program/src/authoring.rs`;
- `crates/kernel/program/tests/authoring.rs`; and
- a focused Runtime hot/cold recovery fixture in `crates/kernel/runtime/tests/runtime_contract.rs`.

Add only `OperationExpansion::with_failure_handler` from Section 2.

#### C2-D.1 Lowering without handler IDs

Nearest-handler semantics do not require a handler target enum or stack. Use local scratch and
exact failure partitions:

1. derive E and J; reject `E == Never` before callbacks;
2. create protected scratch at the caller's current contract, admitting outer failure contracts
   plus E;
3. invoke protected once at checked depth;
4. partition protected `open_failures` by each State's exact failure ref;
5. require at least one reachable exact E slot;
6. require every *open* protected success to be exact J; direct open O is invalid here, while a
   nested already-closed O remains closed;
7. create handler scratch at E with only the outer admitted handlers, so the new handler cannot
   catch itself;
8. invoke handler once at checked depth;
9. require a nonempty State-first handler;
10. classify open exact J as recovered join, open distinct enclosing O as scope success, and any
    other result as invalid;
11. append protected then handler without connecting protected-success J tails to the handler;
12. patch only protected exact-E failure slots to the handler's first State;
13. aggregate protected-success and recovered-handler J tails for the outer continuation;
14. carry all other failures outward unresolved; and
15. merge the whole region once.

This naturally gives same-E nesting: the inner protected partition binds to the inner handler,
while an E failure emitted by that handler remains unresolved and can be partitioned by an outer
same-E region. `E == F` works because exact E is partitioned before root failure finalization.

The handler owns State failure edges only. It does not catch `ProgramError`, `RuntimeError`, Store
or adapter errors, panics, cancellation, or callback failures. There are no backward edges, retry,
finally, compensation, or ambient context.

#### C2-D.2 Tests and architect gate

Prove:

- physical protected/handler/continuation order;
- protected success skips handler;
- exact-E failure targets first handler State;
- recovered J rejoins and distinct handler O terminates;
- direct protected open O rejects; nested closed O composes;
- one-State and multi-occurrence regions;
- E==F, outer-F bypass, foreign failure, and `E = Never` rejection before either callback;
- nested distinct and same-E nearest/fallback behavior;
- handler cannot catch itself;
- empty/Match-first/wrong-input/wrong-J/wrong-failure handler rejection;
- Pure classifier followed by the same `match_join` API, including recovering and terminal arms;
- one nonempty before/original/after Read suffix inside a Match arm within an active exact handler;
  assert its rebased first-State arm entry, suffix failure-to-handler targets, Match join, protected
  completion, and handler-skip continuation indices in the final Program;
- hot/cold recovery/terminal RunViews; and
- callback/late-failure atomicity and 64/65 depth; the boundary fixture must mix a child Operation,
  Match definition and arm, protected and handler callbacks, and injection before/binding/after
  hooks in one active nesting so no scratch scope can reset the shared counter.

Architect C2-D traces every failure slot in same-E nesting and verifies that open protected O is
not accidentally accepted, child-relative success still reopens at its caller, and no policy/route
type was added.

### 6.5 Chunk C2-E — EVM policies and `CollectEvmBalances<K>`

Primary paths:

- `crates/domains/evm/src/lib.rs`;
- its existing inline tests;
- `crates/domains/evm/tests/target_contract.rs`; and
- `crates/domains/evm/README.md`, finalized in C2-H.

Do not add an EVM-to-Runtime/Journal/Store edge. Import only the new Program authoring API through
the crate's existing `mfm-program` dependency.

#### C2-E.1 Exact configured child

Replace `append_balance_fragment` with exactly one public configured Operation:

```rust
/// Deterministically expands one checked EVM balance collection.
pub struct CollectEvmBalances<K: MfmValueTrait> {
    target: EvmPhysicalTarget,
    source_count: usize,
    marker: PhantomData<fn() -> K>,
}

impl<K: MfmValueTrait> CollectEvmBalances<K> {
    /// Constructs one collection expansion with a checked source count.
    pub fn new(
        target: EvmPhysicalTarget,
        source_count: usize,
    ) -> Result<Self, EvmDomainError> {
        if !(1..=EVM_BALANCE_SOURCE_LIMIT).contains(&source_count) {
            return Err(EvmDomainError::Program);
        }
        Ok(Self {
            target,
            source_count,
            marker: PhantomData,
        })
    }
}
```

The target is owned so the public Operation has no lifetime. Fields remain private. Add no getter,
alternate constructor, source-count wrapper, public tag helper, or derive not needed by a current
caller.

Its exact contracts are:

```rust
impl<K: MfmValueTrait> Operation for CollectEvmBalances<K> {
    type Input = EvmBalanceContext<K>;
    type Output = EvmBalanceCollectionCompletion<K>;
    type Failure = EvmBalanceFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<
            Self::Input,
            Self::Output,
            Self::Failure,
        >,
    ) -> mfm_program::Result<()> {
        for _ in 0..self.source_count {
            body.read::<CheckChainIdentity<K>, EvmChainIdentityRead>(&self.target)?;
            body.read::<ReadInitialAnchor<K>, EvmAnchorRead>(&self.target)?;
            body.pure::<SelectBalanceAsset<K>>()?;

            body.match_join::<EvmBalanceAsset<K>, EvmBalanceContext<K>>(|arms| {
                let native = StableId::new("native")
                    .map_err(|_| ProgramError::InvalidContract)?;
                arms.arm::<EvmBalanceContext<K>>(native, |branch| {
                    branch.read::<ReadNativeBalance<K>, EvmBalanceRead>(&self.target)
                })?;

                let token = StableId::new("token")
                    .map_err(|_| ProgramError::InvalidContract)?;
                arms.arm::<EvmBalanceContext<K>>(token, |branch| {
                    branch.read::<ReadTokenDecimals<K>, EvmBalanceRead>(&self.target)?;
                    branch.read::<ReadTokenBalance<K>, EvmBalanceRead>(&self.target)
                })
            })?;

            body.read::<ConfirmBalanceAnchor<K>, EvmAnchorRead>(&self.target)?;
        }

        body.pure::<ConsolidateBalanceCollection<K>>()
    }
}
```

This is the required source shape. Small formatting/private helper differences are allowed only if
they delete code and add no public item. Branch physical order remains native then token; only
Match metadata sorting is generic.

#### C2-E.2 Six explicit identity policies

Implement exactly these pairs:

```text
EvmChainIdentityRead / CheckChainIdentity<K>
EvmAnchorRead        / ReadInitialAnchor<K>
EvmBalanceRead       / ReadNativeBalance<K>
EvmBalanceRead       / ReadTokenDecimals<K>
EvmBalanceRead       / ReadTokenBalance<K>
EvmAnchorRead        / ConfirmBalanceAnchor<K>
```

Each uses:

```rust
type Setup = EvmPhysicalTarget;
type ExpandedInput = EvmBalanceContext<K>;
type ExpandedOutput = EvmBalanceContext<K>;

fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
    setup
        .binding_ref()
        .map_err(|_| ProgramError::InvalidContract)
}
```

Do not override empty hooks. A semantic macro may generate these six explicit impls, but there is
no blanket policy and unsupported State/capability pairs remain compile errors.

#### C2-E.3 Deletion ownership and proof

Once C2-F has moved the sole Portfolio caller, delete the complete base lines 1895–2072 (178
physical lines), including:

```text
append_balance_fragment
pure_state
read_state
start_index
source_count_u16
consolidate_index
base and index closures
native/decimals/token/confirm raw indices
failure_next_index/completion_next_index parameters
direct Vec<Declaration> mutation/imports
direct StateDeclaration/Execution/Match constructor imports
```

Do not merely hide the helper. Remove it from exports, rustdoc, tests, and docs.

Tests prove:

- one source = 9 child declarations; two sources = 17;
- per source, Match is after Select, native and token branch targets are exact, and both rejoin
  Confirm;
- the second source follows first Confirm; final Confirm flows to Consolidate;
- all six Reads carry the target binding and exact capability/intent/evidence ABI;
- source counts 1 and 64 accept, 0 and 65 reject before expansion;
- same Operation type repeats with two owned targets/setups;
- native execution invokes no token adapter/provider/Journal path and token invokes no native path;
- token decimals remains its own Read declaration/evidence checkpoint rather than a composite
  adapter result;
- wrong chain/route binding remains pre-provider `RuntimeError::Internal` with zero provider calls
  and zero append;
- identity policies leave current Program bytes unchanged; and
- no raw declaration/index construction survives in EVM.

Architect C2-E traces the exact `8 * sources + 1` declaration order and all six pairings. It
`BLOCK`s any retained raw helper, public setup/tag helper, conditional Read, consolidated adapter,
extra State, blanket policy, or unjustified positive domain-authoring LOC.

### 6.6 Chunk C2-F — Portfolio root and raw-domain-author deletion

Primary paths:

- `crates/domains/portfolio/src/lib.rs`;
- `crates/domains/portfolio/tests/planning_contract.rs`;
- `crates/app/tests/portfolio_runtime.rs`; and
- EVM's deferred raw-block deletion from C2-E.

#### C2-F.1 Planning boundary and owned root

Keep `plan_snapshot`'s public signature and validation. It still:

- validates selector/config;
- selects targets by strict chain ID;
- derives each route binding for C0 demand; and
- returns `(Program, PortfolioSnapshotInput)` with existing `PortfolioError` ownership.

During that same loop, construct all checked children before calling `expand_program`:

```rust
let child = CollectEvmBalances::<PortfolioContinuation>::new(
    target.clone(),
    collection.request.sources.len(),
)
.map_err(|_| PortfolioError::Program)?;
checked_collections.push(child);
```

No domain constructor runs inside `Operation::expand`, and do not add a conversion from
`EvmDomainError` to `ProgramError`.

Use one private owned root with no constructor/lifetime:

```rust
struct PortfolioSnapshotOperation {
    checked_collections: Vec<CollectEvmBalances<PortfolioContinuation>>,
}

impl Operation for PortfolioSnapshotOperation {
    type Input = PortfolioSnapshotInput;
    type Output = PortfolioSnapshotOutput;
    type Failure = PortfolioSnapshotFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<
            Self::Input,
            Self::Output,
            Self::Failure,
        >,
    ) -> mfm_program::Result<()> {
        body.pure::<InitializePortfolio>()?;

        for child in &self.checked_collections {
            body.pure::<EnterPortfolioCollection>()?;
            body.with_failure_handler::<
                EvmBalanceFailure,
                PortfolioContinuation,
            >(
                |protected| {
                    protected.operation(child)?;
                    protected.pure::<ResumePortfolioCollection>()
                },
                |handler| handler.pure::<MapEvmBalanceFailure>(),
            )?;
        }

        body.pure::<ConsolidatePortfolio>()
    }
}
```

Call:

```rust
let root = PortfolioSnapshotOperation {
    checked_collections,
};
let program = expand_program(entry_point_id()?, &root)
    .map_err(|_| PortfolioError::Program)?;
```

Do not create a public Portfolio Operation wrapper, planner context, layout DTO, or constructor.

#### C2-F.2 Exact physical graph

For each collection, retain:

```text
EnterPortfolioCollection
CollectEvmBalances declarations
ResumePortfolioCollection
MapEvmBalanceFailure
next EnterPortfolioCollection | ConsolidatePortfolio
```

Child success targets Resume. Child failure skips Resume and targets Mapper's first State. Resume
success skips the physically adjacent Mapper. Mapper output is the distinct root O and therefore
terminal; its declared root F failure is also terminal. Do not assume its implementation always
fails.

For two one-source collections the exact declaration indices are:

```text
0       Initialize
1       Enter 0
2..10   child 0
11      Resume 0
12      Mapper 0
13      Enter 1
14..22  child 1
23      Resume 1
24      Mapper 1
25      Consolidate
```

First child failures target 12 and Resume 0 targets 13. Second child failures target 24 and Resume
1 targets 25. Both Mappers are terminal.

The formula stays `2 + 4*C + 8*S`. One collection/two sources stays 22 declarations; one
collection/64 sources stays 518.

#### C2-F.3 Deletions

Delete the complete base Portfolio raw-authoring block at lines 1014–1162 (149 physical lines):

```text
CollectionLayout
portfolio_program
portfolio_pure_state
selected Vec<(&EvmPhysicalTarget, usize)>
cursor
fragment_len
enter/resume/mapper/terminal/capacity forecasts
nominal refs used only to build raw declarations
direct Vec<Declaration> mutation/imports
raw Program/State/Execution imports
```

At the same compile-green boundary, perform the deferred EVM deletions in C2-E. The final tree has
no raw domain author.

Tests prove the exact graph/table above, representative Program bytes/content ref, 64/65 capacity,
all binding refs, native/token/mixed collections, failure mapping, late collection transition, and
hot/cold App equality. Application production Rust/API remains unchanged.

Architect C2-F traces two collections and `BLOCK`s any parent child-size knowledge, borrowed root
lifetime, constructor inside expansion, mapper-behavior assumption, App production helper, or
residual EVM/Portfolio raw authoring.

### 6.7 Chunk C2-G — sole source ingress, private constructors, and test migration

Primary paths:

- `crates/kernel/program/src/lib.rs`;
- new `crates/kernel/program/src/tests.rs` for private raw validation;
- replace/split `crates/kernel/program/tests/contracts.rs` into
  `tests/authoring.rs` and `tests/retained_wire.rs`;
- `crates/kernel/program/tests/ui/removed_program_api.rs` and `.stderr`;
- new `crates/kernel/program/tests/ui/scoped_authoring.rs` and `.stderr`;
- `crates/kernel/runtime/src/assembly/tests.rs`; and
- `crates/kernel/runtime/tests/runtime_contract.rs`.

#### C2-G.1 Constructor ledger and visibility

The base has these external uses:

| Constructor | Program integration | Runtime integration | Runtime unit | Production outside Program |
| --- | ---: | ---: | ---: | ---: |
| `Program::new` | 14 | 16 | 0 | Portfolio 1 |
| `StateDeclaration::new` | 11 | 19 | 2 | EVM 2, Portfolio 1 |
| `Execution::pure` | 10 | 15 | 2 | EVM 1, Portfolio 1 |
| `Execution::read` | 1 | 4 | 0 | EVM 1 |
| `MatchDeclaration::new` | 11 | 4 | 0 | EVM 1 |
| `MatchVariant::new` | 15 | 5 | 12 | EVM 2 |

After migration, make all six `pub(crate)`. Production calls outside Program must be zero. No
`test-support` feature or `cfg(test)` public escape is allowed.

#### C2-G.2 Program-owned versus retained-wire tests

Move direct raw graph-construction/validator cases from external `contracts.rs` into
`src/tests.rs`, wired with `#[cfg(test)] mod tests;`. They may use private constructors because
Program owns those invariants.

Keep malformed, noncanonical, unknown-field/tag, oversized, hostile ordering, and full-string wire
tests outside through `Program::decode_canonical`.

The existing Read/Match full-string golden is deliberately not source-authorable: its State lacks
the matching `ReadState<C>` implementation and its Match selector is not a supported enum. Preserve
its exact bytes as a decode-only retained-wire fixture; do not weaken the DSL to recreate it.

Rebuild ordinary valid Pure/Read/Match authoring fixtures through local `Operation` values and
`expand_program`.

#### C2-G.3 Runtime fixture migration

Rewrite these valid fixtures as local Operations:

```text
zero Program
one Pure State
counting/retry Pure graph
missing root codec graph
supported external/nested/manual Match graphs
root domain-failure graph
blocking Pure graph
foreign zero Program
ordinary Read
Failure=Never Read
association-counting Read
```

Preserve explicit Runtime association cases for both generic external and generic adjacent
selectors; do not collapse them into one generic-selector smoke test. They must still prove the
selector descriptor, exact tags, payload codecs, and target State inputs before Store/provider IO.

Add one source-authored Read fixture whose local nonempty injection policy increments test-only
hook counters. Capture the counters after authoring, then drive Runtime through adapter construction
and Program decode, association, Store load/append, Journal hot/cold qualification, start/resume/
read, and provider entry; the counts must remain unchanged at every checkpoint. This is the
execution-boundary proof that hooks are authoring-only, not merely a Program decode assertion.

Every source-authored test Read pair receives an explicit local identity
`CapabilityInjection<S> for C` impl. A test macro may reduce repeated bodies only with explicit
pair invocations; no blanket impl enters production or tests.

These three intentionally hostile shapes are not expressible through the DSL and must be forged as
canonical Program v2 bytes, then admitted through `Program::decode_canonical`:

- `rejoin_program`: State success and failure directly target one continuation State;
- `unsupported_match_program`: selector is not a supported closed enum; and
- `missing_capability_program`: State/capability pairing violates `ReadState<C>`.

In `runtime/src/assembly/tests.rs`, forge/decode a minimal root-Match Program and inspect public
declarations for unsupported tagging, unknown/missing tags, payload-target mismatch, semantic
mismatch, serialized-shape mismatch, and missing/mismatched codec. Delete Runtime-level empty or
duplicate-arm cases already rejected by Program decode and impossible at association. Runtime
production code remains byte-identical.

#### C2-G.4 Compile-fail boundary

Move the trybuild harness out of deleted `contracts.rs` and into `tests/authoring.rs`; UI files are
not discovered automatically. Keep the invocation explicit:

```rust
#[test]
fn unavailable_authoring_surfaces_do_not_compile() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/removed_program_api.rs");
    tests.compile_fail("tests/ui/scoped_authoring.rs");
}
```

Compile-fail tests prove callers cannot:

- call any of the six raw constructors;
- name/use `ProgramAuthor`, `ForwardLabel`, public labels/routes, raw `select`, `place`, `_to`, or
  `return_success`;
- construct, clone, default, or escape `MatchJoin`/`InjectionWriter`;
- call a nonexistent `InjectionWriter::read`/Match/handler/operation/finish method; or
- obtain a second source-authoring ingress.

Do not create old-name compile fixtures whose only purpose duplicates the manifest scanner.

Architect C2-G reconciles every row in the constructor ledger, verifies that hostile coverage was
moved rather than weakened, and `BLOCK`s an impossible hostile shape made source-authorable, a
test-only public backdoor, or Runtime production churn.

### 6.8 Chunk C2-H — documentation, absence authority, and cumulative gate

Primary paths:

- `docs/design.md`;
- `docs/architecture.md`;
- `docs/portfolio-snapshot.md`;
- `docs/evm-rpc-routing.md` where authoring/binding ownership is described;
- `docs/evm-portfolio-contract-freeze.md` only where authoring ownership is described;
- `crates/kernel/program/README.md` and `Cargo.toml` package description if stale;
- EVM, Portfolio, and directly affected live/App READMEs/rustdoc;
- `RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md` status after implementation;
- `RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md`; and
- `scripts/check-cutover-manifest.sh`.

Do not edit Runtime/Journal/Store/PostgreSQL operational docs when their contract did not change.
Replace Program's stale package description with exactly:

```toml
description = "Typed Operation authoring and immutable Program v2 contracts for MFM"
```

Document:

- Operation as deterministic authoring-only composition;
- State as an execution leaf;
- `OperationExpansion` as the sole private-lowering owner;
- Program as the only persisted graph;
- the same `match_join` for ordinary topology and classified recovery;
- structured exact failure handling;
- injection as topology, never runtime authority; and
- the future Effect/security boundary as still deferred.

#### C2-H.1 Scanner additions

Extend the current manifest ledger/coverage/rules and canaries. Add a narrow
`production-non-program` scope if needed rather than weakening `production`. Prove absence of:

```text
ProgramAuthor
ForwardLabel
public Program authoring label/place/raw select/_to/return_success
raw Program constructor calls outside mfm-program
append_balance_fragment
CollectionLayout
portfolio_program
portfolio_pure_state
domain-local pure_state/read_state declaration helpers
direct declaration-vector authoring in EVM/Portfolio
direct .expand(...) or Operation::expand(...) composition outside mfm-program
direct ::original_binding_ref/::write_before/::write_after calls outside mfm-program
extra InjectionWriter methods
numeric EVM/Portfolio families from Commit 1
```

Rules for generic words such as `label`, `place`, `select`, and `_to` must be owner-qualified. The
direct-callback rule targets call syntax, not trait definitions/impl methods. Hostile tests may name
rejected APIs under exact test scopes; production exports/current docs may not teach them.

For every new rule:

1. add/review the deletion-ledger entry;
2. add the machine rule;
3. add one canary that fails when injected;
4. map every ledger row in the coverage block;
5. recompute ledger/ruleset/coverage digests with the script's exact `awk` extraction; and
6. update the exact inventory count.

Do not add broad `docs/**`, `tests/**`, top-level, or crate exclusions. Do not scan ignored local
`runtime.local.toml`, `setup.local.toml`, or `.hermes/`.

#### C2-H.2 Commit-2 and cumulative architect gates

After every chunk approval, assign one non-author Program/capability architect who did not
implement the integration. They inspect the exact Commit-2 candidate and record a separate
`COMMIT-2 APPROVE` or `BLOCK`. They must trace:

- one repeated configured child with distinct owned targets;
- lexical Pure, exact-pair Read, and one synthetic before/original/after suffix;
- native/token Match declaration and unequal arms into Confirm;
- terminal Portfolio failure handling and synthetic classified recovery;
- two collections through Resume/Mapper/next Enter/Consolidate;
- child-relative success reopening versus root terminal sealing;
- same-E nearest-handler behavior without handler IDs;
- scratch atomicity at a late failure;
- callback depth 64/65;
- one private draft and one `Program::new` call;
- all constructor/deletion ledgers;
- exact Program bytes/content ref; and
- the public/LOC/dependency/dexterity report in Section 12.

The Commit-2 reviewer `BLOCK`s any second representation, public budget breach, stale source
constructor, Runtime/Journal/Store/App production change, new dependency edge, scanner blind spot,
or unjustified positive authoring LOC.

After both commit decisions approve, assign a non-author cumulative reviewer to inspect the exact
two-commit series and record `CUMULATIVE APPROVE` or `BLOCK`. This decision separately checks
Commit-1 identity preservation, Commit-2 Program-byte identity, dependency direction, owner-local
State/Operation/injection change sites, the full deletion ledger, and the final complexity report.
The same reviewer may have reviewed a chunk, but this is a new decision on the complete series;
they must not have implemented either commit.

### 6.9 Commit 2 integration definition of done

Commit 2 is ready only when:

- C2-A through C2-H, the Commit-2 gate, and the cumulative gate are `APPROVE`;
- the final public surface matches Section 2 exactly;
- both domain raw-authoring regions and every external raw-constructor call are gone;
- all source authors use `expand_program` and retained hostile bytes use only
  `Program::decode_canonical`;
- exact Program bytes/schema/content ref and all public result fixtures are unchanged;
- Runtime, Journal, Store, PostgreSQL, App production, CLI/REST, signing, and keystore are unchanged;
- members/edges remain 18/45;
- focused verification and final CI in Section 12 are green; and
- the subject is exactly `centralize operation expansion and capability injection`.

## 7. Mandatory deletion ledger

Deletion is part of correctness. A passing test suite with one superseded path still present does
not implement the RFC.

### 7.1 Commit 1 deletions

Delete from production exports, impls, registrations, tests, and current docs:

```text
EvmCapability<const KIND: u8>
EvmState<const FAMILY: u8, const STAGE: u8, K>
PortfolioState<const STAGE: u8>
all numeric family/kind/stage instantiations
numeric compatibility aliases
ordinal comments and arbitrary numeric macro parameters
```

Retain every stable-ID literal. The absence target is source spelling/authority, not the semantic
identity strings.

### 7.2 Commit 2 Program deletions/privacy

Delete or make private exactly as indicated:

```text
public Program::new                         -> pub(crate)
public StateDeclaration::new                -> pub(crate), return Self if cleaned up
public Execution::pure/read                 -> pub(crate)
public MatchDeclaration::new                -> pub(crate)
public MatchVariant::new                    -> pub(crate)
all external raw-constructor calls          -> delete/migrate
ProgramAuthor                               -> absent
ForwardLabel                                -> absent
public/private label table                  -> absent
label/place/*_to/raw select/return_success  -> absent
public route/successor/address type          -> absent
Operation AST/recipe/node/exit-policy enum  -> absent
dynamic/boxed/stored Operation callbacks    -> absent
test-support constructor feature/export     -> absent
InjectionWriter::read/effect/operation/...  -> absent
blanket CapabilityInjection impl            -> absent
second finalized declaration form           -> absent
```

The private `DraftDeclaration` is the one allowed lowering IR. The existing final
`Declaration::{State, Match}` is the only Program form.

### 7.3 Commit 2 EVM deletions

Delete:

```text
append_balance_fragment
EVM pure_state helper
EVM read_state helper
fragment start/failure/completion index parameters
start/base/offset/consolidate index arithmetic
source-count u16 topology arithmetic
direct Vec<Declaration> mutation
raw Program declaration imports
docs/tests teaching eight-offset manual layout
```

Do not delete the native/token Match, `EvmBalanceAsset<K>`, separate decimals observation,
`EvmPhysicalTarget`, six surviving Read pairings, or behavior functions.

### 7.4 Commit 2 Portfolio deletions

Delete:

```text
CollectionLayout
portfolio_program
portfolio_pure_state
cursor/fragment_len/enter/resume/mapper/terminal/capacity forecasts
selected borrowed target/count layout vector
direct Vec<Declaration> mutation
duplicate nominal-contract derivation used only for raw construction
raw Program declaration imports
docs/tests teaching parent child-size knowledge
```

Retain `plan_snapshot`, selector/config validation, C0 demand creation, target selection,
`PortfolioError`, the five State behaviors, and public request/result fixtures.

### 7.5 Forbidden production changes

The production diff for these paths must be empty:

```text
crates/kernel/runtime/src/**
crates/kernel/journal/src/**
crates/kernel/store/src/**
crates/storages/postgres/src/**
crates/app/src/**
bin/**/src/**
crates/signing/src/**
crates/keystore/src/**
```

Runtime test files may change as prescribed. If a production change appears necessary, stop and
obtain an RFC/plan amendment; do not hide it in a test migration.

## 8. Authoring dexterity proof

The final architect records exact files for each representative change. Expected owner-local
changes are:

| Change | Required owner changes | Must not change |
| --- | --- | --- |
| New Pure State occurrence | domain State type/behavior/ID, containing Operation call, trusted Runtime registration | Program compiler, Runtime fold, Journal, Store, schema, App production |
| New Read with existing capability | domain State behavior, exact pair injection impl, containing Operation call, trusted State registration | Program compiler, Journal, Store, App production |
| New Read capability/target | domain capability/State/target, live adapter constructor/registration, trusted composition | Runtime fold, Journal, Store, App production |
| New reusable Operation | one domain configured value/`Operation` impl and parent `operation` call | Program wire, Runtime, Journal, Store |
| Repeat Operation/State | checked values/setup plus ordinary bounded Rust loop/calls | index/count forecasts anywhere |
| Nonempty injection | exact `CapabilityInjection<S> for C` owner and support Pure State(s) | parent Operation topology/counts, Runtime, Journal, Store |
| New Match-selected topology | selector value plus one containing Operation's `match_join` arms | raw routes/indices, Runtime projection implementation |

The engineer must report deviations. Adding a normal State or injection policy must not require
editing `authoring.rs`; if it does, the compiler has captured domain policy and the architect must
`BLOCK` it.

## 9. Cutover, activation, and rollback

Both commits are source/API cutovers. Program v2 bytes and all run persistence are unchanged. There
is:

- no database migration;
- no Program/Journal compatibility decoder;
- no dual authoring;
- no feature flag;
- no history conversion; and
- no activation order beyond deploying a coherent binary.

Rollback is whole-commit only. Commit 2 depends on Commit 1's semantic names. Do not cherry-pick a
partial chunk or roll back constructor privacy while retaining two authoring paths. Because bytes
are identical, runs created before/after Commit 2 remain readable by either coherent source
version.

A future Effect release is a separate persisted/security cutover and is not a continuation step in
this plan.

## 10. Verification matrix by owner

Use the narrowest checks while iterating. Direct Cargo/Rust commands run only in the default Nix
development shell.

| Chunk | Focused verification |
| --- | --- |
| C1-A | `nix develop -c cargo test -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets` |
| C1-B | `nix develop -c cargo test -p mfm-portfolio -p mfm-app --all-targets` |
| C1-C | all four affected package groups plus `negative-scan` |
| C2-A | `nix develop -c cargo test -p mfm-program --all-targets` with lexical filters while iterating |
| C2-B | Program injection filters, then all Program targets |
| C2-C | Program Match filters, then all Program targets |
| C2-D | Program handler filters and focused Runtime recovery filter |
| C2-E | `nix develop -c cargo test -p mfm-evm -p mfm-evm-live --all-targets` after C2-F closes the compile seam |
| C2-F | `nix develop -c cargo test -p mfm-portfolio -p mfm-app --all-targets` |
| C2-G | `nix develop -c cargo test -p mfm-program -p mfm-runtime --all-targets` |
| C2-H | affected package tests, `capacity-app`, `negative-scan`, docs/link review |

Do not create a commit solely to make one chunk compile. When E/F or G crosses an inseparable seam,
record that fact and run the combined focused command once the seam closes.

## 11. Negative and hostile proof ownership

Use one proof per boundary:

- compile-fail owns Rust visibility/available-method boundaries;
- Program private tests own raw graph-validator invariants impossible through the DSL;
- Program retained-wire integration tests own hostile/noncanonical bytes;
- Runtime assembly tests own structurally valid but unassociated Match/value/capability bytes;
- domain tests own stable IDs, behavior, exact source graph, and result fixtures;
- App tests own hot/cold end-to-end Portfolio behavior;
- the manifest scanner owns retired production exports/calls/docs; and
- exact Program goldens own declaration-order identity.

Do not duplicate retired-name compile fixtures merely to repeat the scanner. Do not delete a
Runtime association hostile test just because source authoring now rejects the same condition;
retained bytes remain an independent ingress. Conversely, delete Runtime cases that Program decode
can never admit, such as empty or duplicate Match arms, when Program already owns them.

## 12. Reproducible complexity and verification evidence

### 12.1 Frozen baseline

At base `599c5a5bf6008b04bb39eaa81c010fc842ffda1c`:

| Measure | Baseline |
| --- | ---: |
| Workspace members | 18 |
| Normal internal MFM dependency edges | 45 |
| Tracked crate/bin `src` Rust LOC, including inline tests | 17,839 |
| External crate/bin `tests` Rust LOC | 5,764 |
| Mechanical top-level public declarations/reexports, excluding `pub mod` | 177 |
| EVM raw authoring block | 178 |
| Portfolio raw authoring block | 149 |
| Combined raw domain authoring pool | 327 |

The public count is a regex cross-check, not the logical API authority. Commit 1 deliberately adds
thirteen semantic type names. The reviewed crate-root export/method ledger owns the exact DSL
budget.

### 12.2 Identical workspace measurements

At base, Commit 1, and final Commit 2, run from a clean worktree:

```bash
nix develop -c sh -lc '
set -eu
metrics_dir=$(mktemp -d)
trap "rm -rf -- \"$metrics_dir\"" EXIT

cargo metadata --format-version 1 --no-deps > "$metrics_dir/metadata.json"

printf "workspace_members="
jq -r ".workspace_members | length" "$metrics_dir/metadata.json"

jq -r "
  [.packages[] as \$package
   | \$package.dependencies[]
   | select(.source == null and .kind == null)
   | select(.name | startswith(\"mfm-\"))
   | [\$package.name, .name]]
  | unique | sort | .[] | @tsv
" "$metrics_dir/metadata.json" \
  | tee "$metrics_dir/normal-edges.tsv"
printf "normal_internal_edges="
wc -l < "$metrics_dir/normal-edges.tsv"

git ls-files \
  ":(glob)crates/**/src/**/*.rs" \
  ":(glob)bin/**/src/**/*.rs" \
  | tee "$metrics_dir/src-files.txt" \
  | xargs wc -l \
  | tail -1

git ls-files \
  ":(glob)crates/**/tests/**/*.rs" \
  ":(glob)bin/**/tests/**/*.rs" \
  | tee "$metrics_dir/test-files.txt" \
  | xargs wc -l \
  | tail -1

xargs rg -n --no-heading \
  "^pub(?:\\s+async)?\\s+(?:struct|enum|trait|type|fn|const|static|use)\\b" \
  < "$metrics_dir/src-files.txt" \
  | rg -v ":pub mod " \
  | tee "$metrics_dir/public-api.txt"
printf "mechanical_public_items="
wc -l < "$metrics_dir/public-api.txt"
'
```

The architect retains the sorted edge list and public declaration list, not only their counts.

The baseline raw-authoring pool is reproducible from the pinned base:

```bash
base=599c5a5bf6008b04bb39eaa81c010fc842ffda1c
git show "${base}:crates/domains/evm/src/lib.rs" \
  | sed -n '1895,2072p' | wc -l
git show "${base}:crates/domains/portfolio/src/lib.rs" \
  | sed -n '1014,1162p' | wc -l
```

Those commands must report 178 and 149. They are baseline evidence only; do not preserve comments
or filler to game a final range.

### 12.3 Commit and selected-authoring LOC evidence

For each commit report production source churn with:

```bash
git diff --numstat <parent>..<commit> -- \
  ':(glob)crates/**/src/**/*.rs' \
  ':(glob)bin/**/src/**/*.rs'
```

For Commit 2, separately ledger every production line in:

- all of `crates/kernel/program/src/authoring.rs`;
- `CollectEvmBalances`, its constructor/Operation impl, and six injection impls;
- `PortfolioSnapshotOperation` and its Operation impl; and
- the changed `plan_snapshot` child/root construction glue.

Make that final ledger reproducible after `rustfmt`, rather than reporting a reviewer estimate.
Record the final Commit-2 SHA and one non-overlapping inclusive line range for every contiguous EVM
or Portfolio block above. Include blank lines, comments, rustdoc, derives, imports used only by the
block, and private glue/helpers; a helper called by a selected block cannot be omitted merely
because it was placed elsewhere. Count all of `authoring.rs`, including any inline `cfg(test)` code
if the implementation disregards this plan and puts tests there. The preferred implementation
keeps substantial tests in the separate test owners named in C2-A.

The handoff ledger has this exact tab-separated shape:

```text
owner<TAB>path<TAB>first_line<TAB>last_line
CollectEvmBalances<TAB>crates/domains/evm/src/lib.rs<TAB><first><TAB><last>
... one row per non-overlapping contiguous selected block ...
```

Replace every placeholder with the post-format line number and reproduce each row and total with:

```bash
set -euo pipefail
final_commit="${FINAL_COMMIT:?set FINAL_COMMIT to the Commit-2 SHA}"
ledger=final-selected-authoring-ranges.tsv
test -s "$ledger"
expected_header=$'owner\tpath\tfirst_line\tlast_line'
IFS= read -r actual_header < "$ledger"
test "$actual_header" = "$expected_header"

program_lines=$(git show "${final_commit}:crates/kernel/program/src/authoring.rs" | wc -l)
domain_lines=0
while IFS=$'\t' read -r owner path first last; do
  [[ "$first" =~ ^[0-9]+$ && "$last" =~ ^[0-9]+$ && "$first" -le "$last" ]]
  lines=$(
    git show "${final_commit}:${path}" \
      | sed -n "${first},${last}p" \
      | wc -l
  )
  expected=$((last - first + 1))
  test "$lines" -eq "$expected"
  domain_lines=$((domain_lines + lines))
  printf '%s\t%s:%s-%s\t%s\n' "$owner" "$path" "$first" "$last" "$lines"
done < <(tail -n +2 "$ledger")
printf 'program_authoring=%s\ndomain_authoring=%s\nselected_total=%s\n' \
  "$program_lines" "$domain_lines" "$((program_lines + domain_lines))"
```

`final-selected-authoring-ranges.tsv` is a handoff attachment, not a permanent repository file.
The Commit-2 architect checks that its ranges do not overlap and that `rg`/diff inspection finds no
selected helper, impl, import, or setup glue outside the ledger. The immutable commit SHA plus exact
ranges makes the count independently reproducible even when later edits move lines on the branch.

Compare that final ledger against the 327-line deleted domain pool. Moving code into a differently
named file does not count as deletion. Also report the entire Commit-2 production Rust diff so code
cannot escape the selected measure.

Target a non-positive selected authoring delta. A positive result is `BLOCK` until the Commit-2
architect:

1. groups every added block by invariant;
2. identifies every public/private type and pass it needs;
3. proves no check duplicates `Program::new` or Runtime association;
4. proves no parent clone, label table, handler ID, wrapper, convenience method, or speculative
   future path remains; and
5. records why the smallest correct implementation is still positive.

Future Effect reuse alone is never a justification. Do not remove tests, validation, rustdoc, or
readability to force a negative number.

### 12.4 Public/dependency/change-site evidence

Record:

- Commit 1's accepted `+13` semantic type-name delta;
- Commit 2's exact five concepts, one root, eleven callables;
- six constructors made private;
- `append_balance_fragment` replaced by one `CollectEvmBalances` type plus one constructor;
- every other public item/method addition/deletion;
- exact normal edge pairs, which remain 45; and
- the Section 8 change-site table with real final paths.

The final method ledger names the exact final path/line for every production caller. Its expected
ownership is:

| Public callable | Current production owner/call site |
| --- | --- |
| `expand_program` | Portfolio `plan_snapshot` after constructing the checked private root |
| `Operation::expand` | `CollectEvmBalances<K>` and private `PortfolioSnapshotOperation` impls |
| `OperationExpansion::pure` | EVM child and Portfolio root State occurrences |
| `OperationExpansion::read` | six EVM child Read occurrences |
| `OperationExpansion::operation` | Portfolio protected collection region |
| `OperationExpansion::match_join` | EVM native/token selector |
| `OperationExpansion::with_failure_handler` | Portfolio per-collection protected region |
| `MatchJoin::arm` | EVM native and token arms |
| `CapabilityInjection::original_binding_ref` | six explicit EVM identity pairings, invoked only by Program's compiler |
| `CapabilityInjection::write_before`/`write_after` | default identity hooks for those six pairings; nonempty use is explicitly synthetic in this cut |
| `InjectionWriter::pure` | explicitly synthetic nonempty-policy proof; no production call in this cut |

If an implementation has another caller, list and justify it; do not silently retain an unused
convenience method. The synthetic recovering-handler and nonempty-injection fixtures are recorded
honestly and do not count as production demand.

Any new dependency edge, public convenience method, setup/route wrapper, or second authoring entry
is `BLOCK`.

### 12.5 Program and domain identity evidence

At both commits capture and compare:

```text
representative canonical byte length = 36,560
representative content digest =
  content:sha256-v1:59c503d1d8faa1a7afc33d9bb0bcad42d053e7cecb8571023bbc0cef42ab4beb
Program schema =
  schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c
one collection / 64 sources = 518 declarations
65 sources = rejected
```

Compare exact declaration JSON, all State/capability/binding refs, C0, and the three retained public
JSON fixture blobs—not only the digest.

### 12.6 Final focused and composed verification

After C2-H is green, run:

```bash
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test \
  -p mfm-runtime -p mfm-evm -p mfm-portfolio \
  -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task capacity-app
nix run .#run -- --task negative-scan
git diff --check
```

Run `nix run .#model-check` only if Nixfied task integration changes. Scanner rule content alone
does not select model-check. No flake edit is expected.

After the final cumulative architect approves this exact candidate, run exactly one:

```bash
nix run .#ci
```

Do not immediately precede CI with redundant `.#check`, `.#test`, `.#test-db`, or broad capacity
envelopes on the same tree. Report every command, result, and omitted command rationale.

## 13. Final handoff record

The engineer's final handoff includes:

1. the exact two commit SHAs/subjects and clean worktree status;
2. one C1-A through C2-H architect decision record plus separate Commit-1, Commit-2, and cumulative
   approvals;
3. resolution of every `BLOCK`;
4. exact stable-ID/Program/C0/fixture identity evidence;
5. constructor/numeric/raw-author/scanner deletion ledgers;
6. members, edge pairs, source/test LOC, public surface, selected authoring LOC, and full Commit-2
   production churn;
7. State/Operation/injection dexterity paths;
8. focused verification and final CI evidence; and
9. the four accepted material uncertainties, with no newly discovered unresolved uncertainty.

No permanent implementation log, approval database, generated metric artifact, or compatibility
manifest is added to production source. Attach the review/evidence record to the PR/task handoff.
