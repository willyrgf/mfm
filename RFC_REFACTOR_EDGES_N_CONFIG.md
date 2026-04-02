# RFC: Refactor Fat Edges and Introduce a Shared Authored-Config Framework

Status: Mostly landed; follow-up remaining

Last updated: 2026-04-02

## 1. Executive Summary

- The largest architectural problem is not the portfolio snapshot workflow itself. The larger problem is that MFM lacks a shared internal pattern for authored configuration ingestion, normalization, build, and execution across workflow families.
- Portfolio snapshot is the clearest current example, but the same structural pattern appears in deploy/configure/validate, publish-docs, and shell-heavy workflow wrappers.
- The repo should converge on a shared authored-config framework built around explicit typed representations, with `BuiltConfig` used only where a workflow has a reusable execution-ready compiled spec:
  - `AuthoredConfig`: human-facing authoring format such as `config.toml` or JSON.
  - `CanonicalConfig`: normalized typed config used for deterministic hashing and planning.
  - `BuiltConfig`: typed validated derived config consumed by execution ops when that boundary is useful.
- `config.toml` should be treated as an authoring format, not as the authoritative hashed runtime input.
- Authoritative persisted and hashed runtime inputs should remain canonical JSON encodings of typed internal structures, with no floats and no secrets.
- Transport boundaries should own file reading, format detection, and TOML parsing. Internal MFM workflows should begin after parsing and immediate conversion to typed structures.
- Ops should remain planning-only. States should remain execution-only. This means initial parse and immediate canonicalization should not move into states.
- Reuse should come from a shared internal framework plus workflow-family-specific canonical and built config types, validators, compilers, and execution graphs.
- The first concrete refactor should be to split portfolio into `portfolio.config.build` and `portfolio.execute`, while extracting shared primitives that other workflow families can reuse.
- The current app-layer and wrapper-layer procedural glue should be reduced by moving config build, artifact emission, and result extraction behind internal MFM ops and reusable states.
- Current repo status:
  - portfolio now has explicit authored/canonical/built execution boundaries, build/report publication, and a strict execute root, but some CLI/app compatibility glue still duplicates transport parsing and feature shaping
  - deploy/configure/validate now has authored/canonical/built config crates plus config-build and execute roots, and the public root family is the primary app/CLI/parity path, but transport parsing still sits at the app edge and the workflow-family typing remains shallow
  - `publish-docs` now uses shared authored/canonical ingress and intentionally stops there
  - wrapper-heavy Aave-origin phase-A orchestration is now internal MFM composition through `aave_v3_origin_stack`, while intentionally retaining the current Nix/Foundry compatibility backend
  - the previous compatibility-root follow-up is now closed in code: the SDK supports planner-payload-sourced child op config, so canonical-input compatibility roots can compose build then execute with the build child authoritative for execute-child input materialization

## 2. Problem Statement

Today, MFM has strong internal execution primitives, but config-driven workflows often enter the system through transport-specific procedural glue instead of through a coherent internal authored-config pipeline.

The sharpest example is the portfolio snapshot flow described in `docs/PORTFOLIO_SNAPSHOT_DETAILED.md`:

`nix wrapper -> CLI -> app services -> root op planner -> internal child ops -> execution states -> app/CLI output extraction`

Most of the execution core is already in a good place. The blur happens around:

- config reading and request shaping
- validation ownership
- compiler/build ownership
- artifact and report extraction
- wrapper- and app-layer duplication

That blur produces fat edges around otherwise solid internal primitives.

This RFC proposes a broader authored-config framework for MFM, with portfolio snapshot as the first adopter rather than the only target.

## 3. Goals

- Reduce procedural glue at transport, app, and wrapper boundaries.
- Move more workflow logic into internal MFM primitives while preserving the thin-layer contract.
- Standardize how config-driven workflows ingest, normalize, validate, build, and execute configuration.
- Make `config.toml` a first-class human authoring surface without making it the authoritative hashed runtime representation.
- Preserve content-addressing and canonical JSON hashing.
- Preserve append-only run streams, per-append atomicity, and deterministic replay semantics.
- Preserve crate boundaries so library code remains usable without the CLI.
- Prefer reusable shared states and shared helpers over workflow-specific glue code where the behavior is genuinely common.
- Create a migration path that works incrementally and does not destabilize public CLI or JSON output contracts.

## 4. Non-Goals

- This RFC does not propose ambient filesystem reads or network access from execution states.
- This RFC does not propose changing canonical JSON hashing rules.
- This RFC does not propose allowing floats in hashed structures.
- This RFC does not propose making binaries thicker or moving workflow logic out of ops and states.
- This RFC does not propose one giant universal compiler that erases workflow-family-specific logic.
- This RFC does not require a big-bang rewrite of existing config-driven flows.

## 5. Design Constraints

The following design rules are non-negotiable and shape every recommendation in this RFC:

- `docs/design.md` remains the authoritative design contract.
- Append-only stream families remain append-only.
- Per-append atomicity remains intact.
- Content-addressing remains intact.
- Canonical JSON remains the only authoritative hashing representation for structured runtime inputs and outputs.
- Hashed structures must not contain floats.
- Secrets must not appear in manifests, events, artifacts, context snapshots, or user-facing outputs.
- No ambient IO in state logic.
- Binaries remain transport-only.
- Ops remain planning-only.
- States remain execution-only.
- Reusable logic should prefer shared states over op-local execution state implementations.
- Library APIs must remain usable without the CLI.

## 6. Terminology

This RFC uses `workflow family` instead of `domain` to avoid ambiguity.

A workflow family is a config-driven application surface that owns:

- an authored config shape
- a canonical typed config
- optionally, a built or compiled config when the workflow has a reusable execution-ready compiled spec
- one or more planning or execution entrypoints that consume those typed surfaces

Examples in this repo:

- portfolio snapshot
- deploy/configure/validate
- publish-docs
- a future Aave-origin build-and-execute workflow, if it is promoted from wrapper scripts into internal MFM flows

The three config stages are:

- `AuthoredConfig`
  - Human-facing config shape, typically loaded from `config.toml` or JSON.
  - May contain aliases, defaults, loose authoring conveniences, and transport-specific shape.
- `CanonicalConfig`
  - Typed normalized config that is deterministic, hashable via canonical JSON, and suitable for planning.
  - No floats in hashed fields.
  - No transport ambiguity.
- `BuiltConfig`
  - Typed validated derived config produced by the workflow-family build pipeline.
  - Intended to be the authoritative runtime input for execution ops.
  - May include derived plans, expanded targets, prevalidated registries, build metadata, and content-addressed artifact references.
  - Not every workflow family needs this stage. Planner-heavy families can stop at canonical config and persist typed plan artifacts until a reusable execution-ready compiled spec emerges.

## 7. Current Diagnosis

### 7.1 Main Diagnosis

MFM already has good execution primitives, but config-driven workflows often bypass them during ingress and build.

The main issue is not that ops and states are weak. The issue is that authoring ingestion and config build semantics are not yet expressed as a first-class internal MFM pattern.

As a result:

- wrappers synthesize request JSON
- CLIs own feature-specific transport logic
- app services own parsing, validation, and result extraction
- root ops decode and validate inputs again
- planners sometimes recompile the same semantic structures more than once
- built artifacts are not consistently modeled as first-class outputs of a config build workflow

### 7.2 Top 5 Current Fat Edges

1. `crates/app/src/lib.rs`, `AppServices::start_portfolio_snapshot`
   - This is the largest fat edge in the portfolio path.
   - It validates inputs, launches the run, decodes final context, fetches artifacts, extracts reports, and shapes the public response.
   - That is too much workflow ownership for the app layer.

2. `crates/app/src/lib.rs`, `parse_portfolio_snapshot_request_input`
   - The app layer currently owns raw request-file semantics for portfolio.
   - This is the wrong place to grow `config.toml` support or broader authored-config logic.

3. `bin/cli/src/commands/portfolio/snapshot.rs`, `execute_internal` and `execute_request_with_starter`
   - The CLI manually rebuilds a feature envelope instead of consistently using the feature catalog path.
   - This duplicates request parsing and output shaping.

4. `crates/ops/portfolio-tracker-op/src/lib.rs`, `parse_config`, `expand`, and `planner_payload`
   - The root op decodes, validates, and compiles the same logical request shape at multiple points.
   - The semantic execution spec is compiled twice for a single request because planner expansion and planner payload generation are separate.

5. `nixfied/project/module.nix`, portfolio task wiring and CI request synthesis
   - The wrapper layer knows too much about request JSON shape and result extraction.
   - This weakens the thin-wrapper rule and makes transport changes ripple outward.

## 8. File-Level Findings

### 8.1 Portfolio Snapshot Flow

- `nixfied/project/module.nix`
  - The packaged portfolio task and CI workflow synthesize or expect canonical request JSON and manually route it through the CLI.
  - This is useful today, but it is still procedural wrapper glue that should shrink once config build becomes internal.

- `bin/cli/src/commands/portfolio/snapshot.rs`
  - `execute_internal` reads raw request JSON or file input and manually constructs a feature result envelope.
  - `execute_request_with_starter` duplicates output shaping that already has a home in the feature layer.

- `crates/app/src/lib.rs`
  - `start_portfolio_snapshot` is the primary app-layer fat edge.
  - `start_portfolio_snapshot_from_request_input` and `parse_portfolio_snapshot_request_input` keep raw transport parsing in the library layer.
  - `FeatureCatalog::execute` is already the right central abstraction for stable feature execution, but the portfolio CLI path partially bypasses it.

- `crates/ops/portfolio-tracker-op/src/lib.rs`
  - `parse_config` reconstructs a typed bundle from ad hoc JSON fields.
  - `expand` compiles the semantic execution plan.
  - `planner_payload` recompiles equivalent plan data for reporting.
  - This is a real blur between input decoding, validation, compilation, and planner-only output shaping.

- `crates/ops/portfolio-tracker-op/src/plan.rs`
  - `DefaultPortfolioPlanCompiler::compile` is good reusable domain logic, but it is stranded in the op crate.
  - That logic belongs behind a reusable workflow-family compiler boundary that can serve build and execute flows.

- `crates/states/portfolio/src/execution_states.rs`
  - The execution states are dense, but they are on the correct side of the boundary.
  - They should remain the execution core.
  - If they are split further, that split should still remain inside shared state crates, not move back upward into app or CLI glue.

- `crates/states/portfolio/src/model.rs`
  - `validate_portfolio_bundle` and related canonical model validation are well placed.
  - This is the kind of logic that should remain reusable and shared.

### 8.2 Other Workflow Families With Similar Seams

- `crates/app/src/lib.rs`, `start_deploy_configure_validate_from_spec_input`
  - The app layer owns raw spec-file reading and JSON parsing for deploy/configure/validate.
  - This matches the same anti-pattern seen in portfolio config ingestion.

- `crates/app/src/lib.rs`, `start_deploy_configure_validate` and feature `pipeline.deploy_configure_validate.start`
  - The public root-op handoff is now clean, but the spec ingestion path remains transport-driven rather than authored-config-framework-driven.

- `crates/ops/evm-deploy-configure-validate-op/src/lib.rs`, `EvmDeployConfigureValidateOp::expand`
  - This op is thin and correctly placed.
  - It now composes the dedicated config-build and execute roots correctly.
  - The remaining gap is not op layering; it is the still-shallow typing at the ABI/args/assertion boundary.

- `crates/tools/publish-docs/src/catalog.rs`, `load_desired_catalog`
  - This tool already consumes TOML and normalizes it into typed structures.
  - That makes it a useful example of the shape the broader framework should standardize, even though it currently lives outside the general MFM run model.

- `nixfied/project/aave-origin-tools.nix`
  - Aave-origin flows are still shell-heavy and wrapper-heavy.
  - If promoted into internal MFM workflows, they should follow the same authored-config build and execute pattern instead of repeating wrapper-level orchestration.

## 9. Why a Portfolio-Only Fix Is Not Enough

A portfolio-only fix would reduce pain in one path, but it would not solve the repo-wide pattern:

- file parsing and authoring-format decisions would remain scattered
- config build semantics would remain workflow-specific glue
- wrapper logic would continue to synthesize request JSON
- app services would continue to accumulate transport responsibilities
- future config-driven workflows would repeat the same mistake

The higher-leverage change is to define a shared internal framework for authored config workflows, then migrate portfolio first and reuse the same pattern across the repo.

## 10. Architecture Options

### Option A: Boundary-Only TOML Adapter

What moves where:

- Add TOML parsing alongside current JSON parsing at CLI and wrapper boundaries.
- Convert TOML to the current request JSON shape.
- Keep portfolio and other workflows otherwise unchanged.

Tradeoffs:

- Lowest migration risk.
- Easy to ship quickly.
- Leaves most procedural glue intact.
- Does not create a reusable internal model for config build workflows.

Fit with current design rules:

- Preserves thin binaries if kept narrowly scoped.
- Does not violate the design contract.
- Does not meaningfully improve reuse.

Migration cost:

- Low.

Long-term payoff:

- Low to medium.

### Option B: Portfolio-Specific Compile and Execute Split

What moves where:

- Introduce `portfolio.config.build` and `portfolio.execute`.
- Keep the broader repo unchanged for now.
- Reuse the current portfolio compiler and states more cleanly.

Tradeoffs:

- Strong improvement for portfolio.
- Good first adopter path.
- Still leaves the repo without a shared authored-config framework.
- Other workflow families will likely reimplement similar build pipelines independently.

Fit with current design rules:

- Strong fit.
- Preserves thin binaries and thin ops.

Migration cost:

- Medium.

Long-term payoff:

- Medium to high for portfolio, medium overall.

### Option C: Shared Authored-Config Framework With Workflow-Family Build and Execute Ops

What moves where:

- Standardize all config-driven workflow families on `AuthoredConfig -> CanonicalConfig -> BuiltConfig`.
- Transport boundaries parse TOML or JSON and convert immediately into typed authored structures.
- Shared internal helpers handle canonicalization rules, artifact emission, result extraction, and stable build-report surfaces.
- Each workflow family owns:
  - authored schema
  - canonical schema
  - built schema
  - validation
  - build or compile logic
  - execution graph
- Each workflow family exposes:
  - `<workflow>.config.build`
  - `<workflow>.execute`

Tradeoffs:

- Highest initial design work.
- Requires careful scoping to avoid an over-generic abstraction.
- Produces the strongest reuse and the cleanest architecture.

Fit with current design rules:

- Best fit with the thin-layer model.
- Best fit with content-addressed artifacts and deterministic planning.
- Best fit with library-first APIs and shared state reuse.

Migration cost:

- Medium to high, but can be phased.

Long-term payoff:

- High.

### Recommendation

Choose Option C.

Portfolio snapshot should be the first adopter, not the final scope.

## 11. Recommended Target Design

### 11.1 Core Model

The shared authored-config framework should standardize the shape of config-driven workflow families, while leaving workflow-specific semantics in workflow-owned code.

Shared across workflow families:

- authoring loader conventions
- immediate authored-format parsing rules
- canonical JSON hashing rules
- secret-safety and no-float checks
- artifact emission helpers
- build-report and export extraction helpers
- common op and state conventions for config build workflows

Workflow-family-specific:

- authored config structs
- canonical config structs
- built config structs
- validation rules
- compiler or build logic
- execution states and graphs
- output schema

### 11.2 Ingestion Pipeline

The standard pipeline should be:

1. Transport layer reads bytes from CLI, REST, Nix task, or other caller.
2. Transport layer detects input format such as TOML or JSON.
3. Transport layer parses bytes into a workflow-family-specific `AuthoredConfig`.
4. A pure library canonicalizer converts `AuthoredConfig` into `CanonicalConfig`.
5. The transport or library entrypoint starts `<workflow>.config.build` with the canonical typed request.
6. `<workflow>.config.build` validates and derives `BuiltConfig`, emits artifacts, and produces a stable build result.
7. `<workflow>.execute` consumes `BuiltConfig` and runs the execution workflow.

This split keeps raw file reading and authoring-format parsing out of ops and states while still making the build workflow itself a first-class internal MFM primitive.

### 11.3 Where TOML Parsing Should Live

TOML parsing should live at the transport boundary or in transport-adjacent shared loaders.

It should not live:

- in CLI-specific workflow logic if the same logic is needed by REST or library callers
- in app services that should operate on typed structures
- in ops
- in execution states

The right pattern is:

- shared transport-facing loader helpers parse TOML or JSON
- those helpers immediately produce workflow-family-specific typed authored structs
- canonicalization follows immediately

### 11.4 Where Normalization Should Live

Normalization to canonical internal types should live in workflow-family-owned pure library code.

It should not initially be modeled as state-machine execution for the common case.

Rationale:

- immediate normalization is deterministic and side-effect-free
- it is needed before build ops even start
- moving it into states would wrongly blur transport ingestion with execution semantics

States become appropriate after canonicalization when the workflow needs a multi-step deterministic build pipeline that benefits from checkpointing, artifact persistence, or reusable output emission.

### 11.5 What Remains Transport-Only

Transport-only responsibilities should be:

- file reading
- format detection
- parsing authored bytes
- CLI or HTTP argument validation
- rendering text or JSON outputs
- wiring env vars and wrapper arguments

Nix and shell wrappers should remain thin:

- invoke the right CLI or library entrypoint
- pass paths, flags, and environment
- avoid synthesizing rich request JSON when that request can instead be authored as `config.toml` or passed as a stable typed build request

### 11.6 What Becomes Reusable Internal MFM Logic

Reusable internal logic should include:

- shared helpers for canonical config artifact emission
- shared helpers for built config artifact emission
- shared helpers for stable build reports
- shared helpers for export extraction from completed runs
- shared build-state patterns for deterministic artifact generation and persistence
- shared result surfaces for config build outputs

Workflow-family-specific build states are acceptable where the build logic is genuinely specific, but the scaffolding around artifact persistence and output publication should be reused.

### 11.6.1 Candidate Shared Primitives

The framework should be concrete about which pieces are shared and which remain workflow-family-owned.

Shared helpers or library primitives:

- authored-format loaders that parse TOML or JSON into typed `AuthoredConfig`
- immediate canonicalization entrypoints that convert authored config into `CanonicalConfig`
- canonical JSON encoding and digest helpers for canonical and built config artifacts
- stable build-result surface types for reporting artifact ids, warnings, and build metadata
- generic result extraction helpers so app and CLI layers do not each decode exported artifacts manually

Shared state or state-helper patterns:

- write canonical config artifact
- write built config artifact
- publish stable build report into context and exports
- publish final exported artifact references for downstream consumers

Workflow-family-specific logic that should not be forced into generic shared states:

- semantic validation rules
- config compilation or build logic
- workflow-specific derived artifact structure
- workflow-specific execution graphs

The intended split is:

- share the transport, artifact, and reporting scaffolding
- keep the semantic compiler and execution meaning owned by the workflow family

This avoids two common mistakes:

- leaving every workflow family to reinvent the same artifact and result plumbing
- over-generalizing the compiler step into a single abstraction that hides important workflow-specific invariants

### 11.7 Authoritative Hashing and Artifact Rules

`config.toml` should remain authoring-only.

Authoritative content-addressed structures should be canonical JSON encodings of typed internal structures:

- `CanonicalConfig`
- `BuiltConfig`
- workflow-family build reports where those reports are intended to be hashed artifacts

The key rule is:

- semantically equivalent TOML and JSON authored inputs should normalize to the same `CanonicalConfig`
- the same `CanonicalConfig` should produce the same `BuiltConfig` for a given compiler version and deterministic environment

Raw authored TOML may be persisted as an optional non-authoritative audit artifact if useful, but it must not be the authoritative hashed input used for execution identity.

### 11.8 Build Op and Execute Op Shape

Each config-driven workflow family should converge on two primary public roots:

- `<workflow>.config.build`
- `<workflow>.execute`

`<workflow>.config.build` should:

- consume canonical typed config
- validate semantic invariants
- derive built config
- persist canonical and built artifacts as appropriate
- emit a stable build result

`<workflow>.execute` should:

- consume built config
- plan child ops and states
- run execution states
- publish execution outputs and final reports

This split allows the repo to move config build concerns into MFM primitives without forcing all build steps into one generic universal compiler.

## 12. Portfolio Snapshot as the First Adopter

### 12.1 Proposed Pipeline

The portfolio snapshot flow should become:

1. CLI, REST, or Nix task reads `config.toml` or JSON.
2. Shared loader parses authored config into `PortfolioAuthoredConfig`.
3. Pure canonicalizer converts it into `PortfolioCanonicalConfig`.
4. `portfolio.config.build` validates the bundle and produces `BuiltPortfolioConfig`.
5. `portfolio.execute` consumes `BuiltPortfolioConfig`.
6. Existing execution states continue to perform observation, assembly, and artifact emission.

### 12.2 What Should Move

- Input-file parsing should move out of `AppServices::start_portfolio_snapshot`.
- Portfolio-specific feature envelope shaping in the CLI should collapse into the common feature execution path.
- The current compiler in `crates/ops/portfolio-tracker-op/src/plan.rs` should become a reusable portfolio build primitive rather than an op-internal helper.
- Portfolio build outputs should be explicit artifacts of `portfolio.config.build`.

### 12.3 What Should Not Move

- `crates/states/portfolio/src/execution_states.rs` should remain the execution core.
- `crates/states/portfolio/src/model.rs` canonical model validation is already well placed and should remain reusable.
- `crates/ops/portfolio-tracker-op/src/plan_ops.rs` is already thin in the right way and should stay planner-focused.

### 12.4 Which Current Edges Are Best Solved by Which Primitive

- Config compiler op:
  - Best for splitting portfolio into config build and execute phases.
  - Best for making built artifacts explicit.
  - Best for establishing the reusable pattern for other workflow families.

- Config normalization states:
  - Not recommended for the initial parse and canonicalization step.
  - Canonicalization should remain pure library logic invoked before the state machine starts.
  - Consider state-based normalization only if a future workflow has a large multi-step deterministic normalization pipeline that needs checkpointing.

- Reusable artifact-building states:
  - Strongly recommended.
  - Shared states or shared state helpers should handle canonical artifact writing, built artifact writing, and stable build-report publication.

## 13. Cross-Workflow-Family Refactor Candidates

The same pattern should be applied beyond portfolio.

### 13.1 Deploy / Configure / Validate

Current seams:

- `crates/app/src/lib.rs`, `start_deploy_configure_validate_from_spec_input`
- `bin/cli/src/commands/run/pipeline.rs`

Recommendation:

- Introduce a dedicated authored-config surface for deploy/configure/validate instead of treating the JSON spec as a transport-only blob forever.
- Add `deploy_configure_validate.config.build` if the workflow grows richer typed compilation needs.
- Keep `EvmDeployConfigureValidateOp` thin and consume `BuiltConfig` once the build step exists.

### 13.2 Publish Docs

Current seams:

- `crates/tools/publish-docs/src/catalog.rs`, `load_desired_catalog`

Recommendation:

- Treat publish-docs as a strong candidate for the same authored-config framework.
- It already has a TOML authored surface and typed normalization logic.
- The current workflow most naturally stops at canonical wave/catalog config plus typed plan artifacts rather than at a separate built execution config.
- The persisted `plan.json` boundary is the useful downstream contract today; it already captures the planner result derived from canonical config plus live workspace and remote observations.
- Only add a `build -> execute` split if publish-docs later grows a deterministic reusable execution-ready compiled spec that is meaningfully separate from its live planner outputs.

### 13.3 Aave-Origin and Similar Wrapper-Heavy Flows

Current seams:

- `nixfied/project/aave-origin-tools.nix`
- raw `nix_app(fetch) -> nix_app(compile) -> nix_app(deploy) -> aave_v3_origin_adapt_deploy`
  orchestration in the parity scenario
- reusable Aave manifest and runtime state substrate already living in `crates/states/aave-v3`

Recommendation:

- This migration is now landed as `aave_v3_origin_stack`.
- Aave-origin follows the same authored-config build and execute split rather than wrapper-level
  composition.
- It remains a stronger `BuiltConfig` candidate than `publish-docs` because its wrapper-backed
  phase-A topology is deterministic: `fetch -> compile -> deploy -> adapt`.
- The current backend still preserves the existing external Origin tools; internal MFM build and
  execute roots now own orchestration while Foundry compile/deploy behavior remains external.
- Existing Nix task apps remain compatibility surfaces over the same backend.

### 13.4 Generic Result Extraction

Current seams:

- portfolio-specific result extraction in `crates/app/src/lib.rs`
- CLI-local output shaping in `bin/cli/src/commands/portfolio/snapshot.rs`

Recommendation:

- Extract stable shared build-result and execute-result extraction helpers so workflow families do not each reinvent final report decoding.

## 14. Compatibility and Migration

### 14.1 CLI Compatibility

The CLI should preserve current JSON output stability.

Migration plan:

- keep existing JSON request arguments temporarily
- add `config.toml` support as a new authoring entrypoint
- route both JSON and TOML through the same typed canonicalization path
- keep feature output JSON stable while internal build and execute boundaries change

### 14.2 JSON Request Compatibility

JSON request files should remain supported during migration.

The compatibility rule should be:

- both JSON and TOML authoring inputs normalize into the same `CanonicalConfig`
- the rest of the system should no longer care which authoring format was used

### 14.3 Library Compatibility

Library-first APIs should improve, not regress.

Recommended direction:

- add typed entrypoints that accept canonical or built configs directly
- keep transport-specific file parsing out of long-term library APIs

### 14.4 Wrapper Compatibility

Nix and shell wrappers should continue to work during migration.

Short term:

- wrappers can keep invoking existing CLI commands
- wrappers may continue to pass JSON request files while new build surfaces are introduced

Medium term:

- wrappers should prefer `config.toml` or stable config paths
- wrappers should stop synthesizing rich JSON envelopes when that logic can be internalized

## 15. Phased Implementation Plan

### Phase 1: Extract Shared Portfolio Canonical and Build Boundary

Code areas touched:

- `crates/app/src/lib.rs`
- `crates/ops/portfolio-tracker-op/src/lib.rs`
- `crates/ops/portfolio-tracker-op/src/plan.rs`
- portfolio model and compiler code

Change:

- Introduce explicit portfolio authored, canonical, and built config boundaries.
- Move reusable compiler logic behind a stable portfolio build boundary.

Contract risk:

- Low to medium.

Tests and docs:

- add equivalence tests for semantically identical authored inputs
- add canonical hashing tests
- document the new portfolio config stages

Ship independently:

- Yes.

### Phase 2: Add Shared Authored-Config Loader Conventions

Code areas touched:

- CLI entrypoints
- app-facing typed loaders
- wrapper-facing docs

Change:

- Add shared TOML and JSON authored-config loading conventions.
- Convert immediately into typed authored structs and then canonical structs.

Contract risk:

- Medium, because canonicalization order affects digests and artifacts.

Tests and docs:

- TOML versus JSON equivalence tests
- failure tests for float rejection and secret-bearing invalid surfaces
- update CLI docs and portfolio docs

Ship independently:

- Yes.

### Phase 3: Introduce `portfolio.config.build`

Code areas touched:

- portfolio op crate
- shared state or state-helper layer
- feature catalog and app service surface

Change:

- Make portfolio config build a first-class internal MFM workflow.
- Persist canonical and built config artifacts explicitly.

Contract risk:

- Medium.

Tests and docs:

- build artifact stability tests
- build-report schema tests
- docs for portfolio build versus execute responsibilities

Ship independently:

- Yes.

### Phase 4: Slim Portfolio Execute Path

Code areas touched:

- `bin/cli/src/commands/portfolio/snapshot.rs`
- `crates/app/src/lib.rs`
- possibly shared result extraction helpers

Change:

- Stop CLI-local feature envelope rebuilding.
- Remove portfolio-specific app glue that duplicates build and report extraction responsibilities.

Contract risk:

- Low to medium.

Tests and docs:

- CLI feature parity tests
- output schema regression tests

Ship independently:

- Yes.

### Phase 5: Generalize the Framework to Other Workflow Families

Code areas touched:

- deploy/configure/validate ingress
- publish-docs if brought under the shared pattern
- wrapper-heavy workflow surfaces such as aave-origin tooling

Change:

- Apply the same authored-config framework to other workflow families with the same shape.
- Adopter status after the landed work to date:
  - portfolio:
    - explicit authored/canonical/built types, config-build, execute, and compatibility roots are landed
    - shared canonical/built artifact publication and stable build-report plumbing are landed
    - residual work remains to thin CLI/app request parsing and feature-envelope glue
  - deploy/configure/validate:
    - authored/canonical/built config crate plus config-build and execute roots are landed
    - the public root family is now the primary app/CLI surface and direct parity coverage exercises it
    - residual work remains to thin transport parsing at the app/CLI edge
    - the workflow-family config remains only partially typed because deploy/configure/validate payloads still include raw JSON ABI, arg, and assertion seams that should be strengthened
  - publish-docs:
    - authored/canonical ingress is landed
    - no built-config split is intended at this stage because the current tool does not have a reusable execution-ready compiled spec distinct from its typed plan/output surfaces
  - wrapper-heavy Aave-origin tooling:
    - explicit authored/canonical/built/execute boundaries are landed through `aave_v3_origin_stack`
    - the backend intentionally still uses the current Nix/Foundry compatibility wrappers rather than a full Rust reimplementation
  - compatibility-root follow-up:
    - the previous migration seam is now closed
    - the SDK supports planner-payload-sourced child op config, and portfolio, deploy/configure/validate, and Aave-origin compatibility roots now wire execute-child config from the build child's planner payload instead of precomputing built config in the parent planner for execute-child handoff

Contract risk:

- Medium, but distributed across small reviewable changes.

Tests and docs:

- per-workflow-family authored-config equivalence tests
- docs for the shared framework and each adopter

Ship independently:

- Yes.

### Remaining Work Before This RFC Should Be Considered Fully Complete

- Finish deploy/configure/validate adoption:
  - strengthen workflow-family typing beyond raw JSON ABI, arg, and assertion seams
  - thin the remaining transport parsing edge in the app/CLI ingress path

- Thin the remaining portfolio transport edges:
  - reduce duplicated request parsing/canonicalization glue between the CLI and app service entrypoints
  - remove duplicate feature-envelope shaping where the app feature layer can own it directly

- Retire stale compatibility-root follow-up prose:
  - this RFC no longer treats planner-authoritative build-child handoff as remaining work
  - the remaining task is to update workflow docs and inventories that still describe the pre-SDK compatibility seam

- Keep workflow docs synchronized with the landed code:
  - update workflow-specific docs when parity entrypoints or migration status change so the code remains the authority and the docs remain trustworthy summaries

### Completion Checklist

- [x] Portfolio authored/canonical/built boundaries are landed behind config-build, execute, and compatibility roots.
- [x] Deploy/configure/validate authored/canonical/built boundaries are landed behind config-build, execute, and compatibility roots.
- [x] The public deploy/configure/validate root family is the primary app/CLI/parity path.
- [x] Portfolio, deploy/configure/validate, and Aave-origin compatibility roots now source execute-child config from build-child planner payloads.
- [x] Shared artifact/report/export publication plumbing is reusable shared state, not duplicated per adopter.
- [x] Publish-docs uses authored/canonical ingress and intentionally stops before a built/execute split.
- [ ] Strengthen deploy/configure/validate typing beyond raw JSON ABI, arg, and assertion seams.
- [ ] Thin the remaining portfolio CLI/app parsing and feature-envelope glue.
- [ ] Thin the remaining deploy/configure/validate app/CLI transport parsing glue.
- [ ] Update stale workflow docs and inventories that still describe the old compatibility-root seam or outdated adopter status.

## 16. Open Questions and Risks

- Should `BuiltConfig` always be the authoritative runtime input, or are there workflow families where execution should still consume `CanonicalConfig` directly?
- How much shared framework surface is enough before the abstraction becomes too generic and hard to reason about?
- The current planner model still separates `expand()` and `planner_payload()`. Some duplicate compile/report work may remain in individual ops, but the specific child-config handoff seam is no longer blocked on SDK evolution.
- If raw authored TOML is persisted as an audit artifact, what retention and secrecy rules should apply to avoid leaking sensitive but non-secret authoring context?
- Which workflow families are worth bringing fully under the run model versus leaving as standalone tools?
- Doc drift remains a risk as adopter status evolves. For example, workflow-specific migration docs can lag the code and parity tests unless they are updated as part of the same change.

## 17. Specific Answers to the Original Review Questions

### What are the top 5 current fat edges in the portfolio snapshot flow?

1. `crates/app/src/lib.rs`, `AppServices::start_portfolio_snapshot`
2. `crates/app/src/lib.rs`, `parse_portfolio_snapshot_request_input`
3. `bin/cli/src/commands/portfolio/snapshot.rs`, `execute_internal`
4. `bin/cli/src/commands/portfolio/snapshot.rs`, `execute_request_with_starter`
5. `crates/ops/portfolio-tracker-op/src/lib.rs`, `parse_config`, `expand`, and `planner_payload`

### Which are best solved by a config compiler op, config normalization states, or reusable artifact-building states?

- Best solved by a config compiler op:
  - portfolio ingress and compilation ownership
  - explicit build artifact publication
  - separation between build and execute

- Best solved by reusable artifact-building states:
  - canonical artifact emission
  - built artifact emission
  - stable build-report publication

- Not best solved by config normalization states:
  - immediate parse-to-canonical normalization should remain pure library logic

### Should `config.toml` be parsed only at the transport boundary and then converted immediately into canonical typed structures?

Yes.

That is the recommended default architecture.

The only extension to that rule is that workflow-family build ops should then turn canonical typed structures into built typed structures using internal MFM primitives.

### How should reading, validating, normalizing, and artifact-building from config be split?

- CLI, REST, Nix, shell:
  - read bytes
  - detect format
  - parse authored config
  - render outputs

- shared library or workflow-family library:
  - normalize authored config into canonical typed config
  - apply immediate deterministic validation

- config build ops:
  - perform semantic validation
  - derive built config
  - emit artifacts and build reports

- execute ops:
  - consume built config
  - plan runtime execution

- states:
  - perform deterministic build steps where stateful artifact publication helps
  - perform runtime execution

### Which paths are duplicating feature dispatch, request parsing, validation, or output shaping?

- portfolio CLI duplicates feature envelope shaping
- app services duplicate transport parsing for portfolio and deploy/configure/validate
- portfolio root op duplicates validation and compilation work that should sit behind a clearer build boundary
- Nix wrappers still know too much about authored request JSON

### Which parts are already well placed and should not move?

- shared portfolio model validation in `crates/states/portfolio/src/model.rs`
- thin portfolio planner wiring in `crates/ops/portfolio-tracker-op/src/plan_ops.rs`
- execution logic in `crates/states/portfolio/src/execution_states.rs`
- thin composition in `crates/ops/evm-deploy-configure-validate-op/src/lib.rs`

### What is the smallest first refactor with architectural leverage?

Extract one explicit workflow-family build boundary for portfolio:

- define authored, canonical, and built portfolio config types
- move reusable compiler logic behind that boundary
- introduce `portfolio.config.build`
- keep existing execution states largely intact

That is the smallest change that creates leverage for a repo-wide authored-config framework instead of a portfolio-only workaround.

## 18. Recommended Decision

MFM should adopt a shared authored-config framework for config-driven workflow families.

The framework should standardize:

- authoring format ingestion
- canonicalization
- typed built config generation
- artifact emission
- stable build and execute result surfaces

Portfolio snapshot should be the first adopter.

The target end state is:

- humans and AI agents author `config.toml`
- transport boundaries parse authored config into typed structures
- internal MFM build ops produce authoritative canonical and built artifacts
- execution ops consume built config
- wrappers and app services become thinner because the build and execution semantics are represented explicitly inside MFM
