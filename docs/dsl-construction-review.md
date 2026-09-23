# DSL construction and boundary review implementation

Baseline: `05f74977` on `refactor/dsl-phase-b`. The user-approved review is the attachment
`c325b6d4-64fe-4c26-bf58-dbd8109276c7/pasted-text-1.txt`. This record covers its complete five-step
implementation sequence, not a narrowed follow-up to the earlier passing Phase B assessment.

## Required sequence

1. Installed Inventory is the sole owner for fresh/cold executable association. Draft emits only
   declarations, public bindings and policy, checking selected owners against installed contracts.
   Bound handlers decode available parameters during construction; validate every selected parameter
   before native resource binding. Retain generic ABI distinctions and diagnostic causes.
2. One Portfolio collection-set validator owns count/correlation/source aggregate invariants for
   demand and output decoding, retaining local child and output/report checks.
3. Seven infallible Portfolio States declare Never; consolidation has its precise original.
   Product presentation keeps only collection/consolidation summaries, without executable identity,
   classifier or impossible dispatch branches.
4. Read has one completion callback: bind native evidence once, interpret the typed result and
   encode its outcome. Preserve Bind versus Interpret failures and exact originals. Effect retains
   separate callbacks across acknowledgement.
5. Complete the existing failed standalone deployment after restoring resources, on the same RunId,
   retained command and failure prefix. Remove duplicated Pure consumer coverage, name callback fault
   modes, and test manual yield versus automatic continuation after exact-candidate reconciliation.

Each step is one coherent commit with its regression and current documentation. Focused pinned Nix
verification precedes affected managed acceptance and final exact-candidate CI. Final review and
production/test/docs LOC accounting follow all five steps.

## Material uncertainties

Current fixtures may compile semantics absent from their installed environment; these must publish
actual support rather than preserve fresh-only ownership. Validate with complete affected Program,
Runtime and consumer tests, including identity/empty sources and non-Clone parameter contracts.

## Initial evidence

The old claim-conflict test selected both owners in its source and therefore missed source versus
installed disagreement. Selecting only ShadowAdd against installed Add reproduces the defect:

```sh
nix develop -c cargo test -p mfm-program --test claim_conflicts fresh_and_cold_construction_reject_conflicting_state_and_handler_owners -- --exact
```

Failed at the expected `compile(...).is_err()` assertion: fresh compilation accepted the shadow.
The corrected scenario remains the regression. No State evaluation is needed to reproduce it.

## Step 1: installed ownership and checked handlers

Fresh and cold construction now share `Inventory::associate`. Draft retains declarations and
checks selected owners against immutable installed contracts; it no longer owns callbacks or binds
resources. Full execution ABI participates in State ownership. Public native bindings and handler
parameters are decoded before any resource binding; private temporary binders retain those typed
values without decoding twice. Bound handlers capture non-Clone parameters and never decode during
recovery. No persistent schema or additional public registry is introduced.

The shadow-owner regression now asserts `select_state`/`select_handler` and `conflicting_owner`.
New construction coverage rejects schema-valid, typed-invalid handler parameters and later native
bindings before *any* resource binder, on fresh and cold paths. It checks one decode per construction,
non-Clone parameters, and handler decoder panic containment. Existing generic ABI, native cold
construction and recovery-scope coverage remains. Application diagnostics now install the actual
Pure State when testing checkpoint rejection; duplicate installed owners fail during discovery,
before an occurrence has a position.

Pinned verification (all passed):

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --all-targets --message-format short
nix develop -c cargo test -p mfm-runtime --all-targets --message-format short
nix develop -c cargo test -p mfm-app --lib construction_causes --message-format short
nix develop -c cargo check --workspace --all-targets --message-format short
nix develop -c cargo clippy -p mfm-program -p mfm-runtime --all-targets --message-format short -- -D warnings
```

The Runtime classification compile-fail diagnostic was reviewed and updated for the added installed
`Discover` bound, then passed in the normal Runtime suite. Architect review found no ownership
blocker; its request for precise shadow-owner assertions was incorporated. Native binding decoder
panic containment is not a newly claimed guarantee. Managed acceptance and final CI remain pending
until the complete five-step candidate.

## Step 2: aggregate validation

Step 1 revision: `8c83ce25`. Its accounting against `05f74977` is production +254/-197 (net +57),
test Rust +396/-17 (net +379), documentation +102/-6 (net +96), UI diagnostics +24/-0.
The necessary production increase implements selected-owner checks and full preflight before binding;
it deletes the independent hot association authority and repeated recovery parameter decoding.

One private collection-set validator now owns count, correlation uniqueness, total-source bound and
source uniqueness for admission and output. Deleted the separate summation and output source-ID
validator, including owned source-ID copies. Local child checks and output/report agreement remain.
The existing maximum-field scenario now mutates output wire independently: each child still decodes,
but duplicate correlations, cross-child duplicate source IDs and two valid 64-source children fail
aggregate decoding. Re-running the new test against the preceding production implementation failed
at the expected output-rejection assertion; the corrected implementation passes.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-portfolio --all-targets --message-format short
```

Passed all four unit tests and the independent native ABI consumer integration test (both maintained
Portfolio continuations). Production change is +41/-44 (net -3); regression change +51/-0.

## Step 3: exact originals and product summaries

Step 2 revision: `07f1d6a2`. Seven Portfolio States now declare `Never` and consolidation alone
owns `PortfolioConsolidationFailure::AggregateCapacityExceeded` (Permanent). The product summary
loses unused InvalidInput, executable schema identity and classification. Deleted both native-client
dispatch macros, seven impossible branches and the generic product-original decoder. The one
consolidation decoder derives input/failure contracts from its owning State. No compatibility reader
is retained: changed failure ABIs select the current exact contracts.

The existing actual arithmetic failure scenario now decodes the precise consolidation original,
checks unchanged product rendering, and compares exact retained originals and publication hot/cold.
Infallible State consumers use irrefutable success patterns, deleting impossible test panic branches.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo check --workspace --all-targets --message-format short
nix develop -c cargo test -p mfm-portfolio --all-targets --message-format short
nix develop -c cargo test -p mfm-evm-live --lib client::portfolio --message-format short
nix develop -c cargo test -p mfm-app --lib --message-format short
nix develop -c cargo clippy -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets --message-format short -- -D warnings
```

All passed: Portfolio four unit/one integration, native client six, Application fourteen. Initial
workspace checking identified the now-irrefutable test branches; these were removed and Clippy
passed without warnings. Production +48/-67 (net -19), tests +15/-54 (net -39), before documentation.

## Step 4: one Read completion

Step 3 revision: `1ebae644`. Read now has one completion callback which binds/projects native
evidence once, directly interprets its typed result and encodes the outcome. Removed the separate
binding callback and duplicate captures/projection. The internal kernel-only `ReadCompletionFailure`
distinguishes Bind and Interpret while carrying existing `CallbackFailure` unchanged; Runtime keeps
its existing operation diagnostics. Effect callbacks and acknowledgement boundaries are unchanged.

The existing native callback scenario now counts exactly one projection while asserting retained
native evidence and semantic output. Existing fault cases retain binding/interpretation provenance,
codec phases, original acknowledgement timing and cold inspection. No callback authority moved.

```sh
nix develop -c cargo fmt --all
nix develop -c cargo test -p mfm-program --test native_callbacks --message-format short
nix develop -c cargo test -p mfm-runtime --test callback_phases --message-format short
nix develop -c cargo test -p mfm-program -p mfm-runtime --all-targets --message-format short
nix develop -c cargo clippy -p mfm-program -p mfm-runtime --all-targets --message-format short -- -D warnings
```

All passed, including the normal compile-fail checks and Runtime cancellation, append/reconciliation,
recovery, classification and original-custody suites. Production +38/-39 (net -1), tests +27/-9
(net +18), before documentation. The narrow result distinction adds provenance without a second
error framework or evidence cache.
