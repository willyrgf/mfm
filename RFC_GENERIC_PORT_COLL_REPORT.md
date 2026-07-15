# RFC: generic portfolio collect-then-report planning

Status: discussion

Date: 2026-07-15

## Summary

MFM has separate typed operations for collecting facts and for producing a portfolio report. The
`mfm.portfolio/collect_then_report@1` entry point currently composes Bitcoin and EVM native-balance
collectors before invoking the report operation. That composition is useful because it gives callers
one certified workflow and prevents the report from running before collection is complete.

The current composition is only partially generic, however. Its graph is shaped around two
collector families and native balances. A portfolio with only EVM configuration works, and a
portfolio with only native balances works, but the operation does not yet derive a complete
collection plan from every configured resource. Adding token or protocol-position collection would
otherwise require another hard-coded branch or could accidentally leave configured requirements
uncollected.

This RFC proposes that the composed operation become a configuration-driven portfolio collection
planner. It must execute exactly the supported collection requirements implied by the normalized
portfolio configuration, then invoke the existing report operation after a typed readiness barrier.
Configured resource families remain optional. Configured but unsupported requirements fail closed;
they are never silently skipped.

This RFC is a design discussion only. It does not change the current implementation.

## Problem situation

### The existing operations have intentionally narrow responsibilities

The current operation boundaries are correct in isolation:

- resource collector operations observe a specific resource, write source-near facts, and return
  typed batch summaries;
- the portfolio tracker operation is report-only and selects acceptable facts before producing the
  snapshot and report;
- portfolio states own reusable subject resolution, holding selection, valuation resolution,
  snapshot assembly, and report projection;
- binaries and HTTP handlers should not assemble these graphs themselves.

The missing responsibility is the workflow-level plan between a complete portfolio configuration
and those existing operations. A caller needs one deterministic answer to:

1. Which configured resources require collection?
2. Which certified collector operation handles each resource?
3. What evidence proves that every required collection completed?
4. When is it safe to start the report graph?

Without that plan, every caller must understand the relationship between networks, wallets, symbols,
reader kinds, collector policies, and report readiness. That creates multiple opportunities for
different callers to collect different subsets of the same portfolio or to run a report over
partial data.

### Optional configuration is a normal case

The operation must not assume that every resource family is present. These are valid configurations
when their requirements are supported:

- one or more EVM networks with native balances and no Bitcoin network;
- one or more Bitcoin networks with native balances and no EVM network;
- a dual-network portfolio;
- a native-balance-only portfolio with no token symbols;
- a portfolio containing native balances and supported token or protocol resources.

The absence of a resource family means that no child collector should be planned for that family.
It must not be represented as a failed or empty collection branch.

### Configured resources must not disappear silently

The opposite case is more serious: a portfolio can contain a symbol or reader configuration for
which the composed operation has no collector. Treating that resource as absent would allow the run
to succeed while producing a report that does not represent the requested portfolio.

The planner therefore needs to distinguish:

| Configuration condition | Required behavior |
|---|---|
| Resource is not configured | Do not create a collection branch. |
| Resource is configured and supported | Create and execute the matching typed collector branch. |
| Resource is configured but unsupported | Fail planning with an explicit unsupported-requirement error. |
| No collectible resource is configured | Fail with an explicit empty-collection result, unless a future product decision defines an intentional empty report. |

The default must be fail-closed. A future degraded or reuse-existing-facts mode would need to be an
explicit policy, not an accidental consequence of missing collector support.

## Goals

- Derive a complete collection plan from validated portfolio configuration and explicit collection
  policy.
- Run only the collector branches required by that plan.
- Support zero, one, or many configured resource families without special-casing the caller.
- Treat native-only portfolios as complete and valid when their native requirements are supported.
- Add token and protocol-position collection by adding supported collector mappings, not by changing
  the CLI, REST layer, or report states.
- Reject configured-but-unsupported requirements before any child collection starts.
- Prove that every planned requirement returned an acceptable typed result before starting the report
  graph.
- Preserve deterministic graph construction, certification, evidence, replay, and provenance.
- Keep existing collector operations and the report-only operation independently reusable.

## Non-goals

- Making the report operation perform live collection.
- Moving resource-specific observation or fact-writing logic into the composition crate.
- Creating a new crate for every token, protocol, or network family.
- Adding a dynamic plugin or untyped runtime operation registry.
- Treating every field in a portfolio configuration as automatically collectable without a declared
  collector contract.
- Returning a successful partial report when a required collection is missing or unsupported.

## Proposed solution

### 1. Derive a typed collection requirement set

After the app resolves the exact catalog-backed `PortfolioConfig`, the composed operation validates
the portfolio joins and derives an operation-owned collection requirement set.

Each requirement should identify, at minimum:

- the network identity and network family;
- the wallet subjects or subject batch;
- the symbol or resource identity;
- the configured reader/resource kind;
- the collector operation kind and version selected for that reader;
- the expected coverage and source-read policy;
- a deterministic requirement key used for ordering, readiness, and evidence.

The requirement set is derived from semantic configuration. It is not stored in runtime config and
does not contain endpoints, credentials, signer material, or secret-bearing paths.

The planner should enumerate all configured collectable readers, not only the native reader. For a
native-only portfolio, enumeration naturally produces only native requirements. For a portfolio
with supported token readers, it produces native and token requirements. The absence of token
configuration produces no token branch.

### 2. Use closed, typed collector dispatch

"Generic" here means generic over the supported configuration and collector contract. It does not
mean that arbitrary untrusted code is discovered at runtime.

The operation should use a closed, typed mapping from supported reader/resource kinds to existing
collector operations. Each mapping supplies:

- the typed child configuration;
- the typed child operation call;
- the typed summary or collection result;
- the coverage and anchor checks required before readiness.

The mapping is part of the certified program surface. Adding a new resource kind adds its typed
collector mapping and state descriptors while leaving the caller-facing workflow unchanged.
Unsupported reader kinds produce an explicit planning error before the graph is expanded.

This preserves the repository's strong typing and certification model while avoiding a second
dynamic operation-dispatch mechanism.

### 3. Expand one child branch per deterministic requirement or valid batch

The operation expands the collection plan into child scopes. It may batch compatible requirements
when the collector contract requires a shared network anchor, but batching must be deterministic and
must not merge incompatible network, reader, or policy requirements.

The conceptual graph becomes:

```text
validated portfolio + collection policy
  -> deterministic collection requirement set
  -> typed collector child branches
  -> typed readiness fan-in over the exact requirement set
  -> existing report-only portfolio operation
      -> ResolveSubjects
      -> SelectHoldings
      -> ResolveValuations
      -> AssembleSnapshot
      -> ProjectReport
```

The composition operation owns only the first three stages and the call into the report operation.
It reuses the existing collector and report operations through typed operation calls; it does not
reimplement their states.

### 4. Make readiness requirement-based rather than family-count-based

The current readiness shape counts Bitcoin and EVM summaries. A generic planner needs readiness to
prove the exact planned set, not merely that an aggregate family count matches.

The readiness barrier should verify:

- every expected requirement has exactly one result;
- no unexpected or duplicate requirement result is present;
- each result has non-empty, acceptable coverage;
- network anchors satisfy the collector and report selection contract;
- failures are reported before report states execute.

Only after this proof should the operation call the existing report operation with its typed
readiness input. The report remains fact-backed and report-only; the readiness result is an execution
dependency, not a second report authority.

## Configuration examples

| Portfolio configuration | Planned collection graph | Expected outcome |
|---|---|---|
| EVM native only | EVM native branches only | Collect EVM facts, then report. |
| Bitcoin native only | Bitcoin native branches only | Collect Bitcoin facts, then report. |
| EVM and Bitcoin native | Both family branches | Collect all required facts, then report. |
| EVM native plus supported token | EVM native plus token branches | Collect both resource kinds, then report. |
| EVM native plus unsupported token reader | No successful plan | Fail with unsupported requirement; do not produce a partial report. |
| No networks or collectible subjects | No child branches | Fail with empty collection by default. |

An EVM-only run must not fail because the Bitcoin family is absent. A native-only run must not
fail because token collector support is absent when no token is configured.

## Error and partial-execution semantics

Planning errors occur before child expansion whenever possible:

- invalid portfolio or wallet/symbol/network join;
- configured reader with no supported collector;
- missing required collector policy;
- incompatible batching or missing network identity;
- empty collection requirement set.

Once execution begins, a required child failure fails the composed workflow. The operation must not
invoke the report graph with a partial readiness result. Collector facts that were already written
remain subject to the existing append-only and replay/evidence rules; the composed run itself does
not convert a failed collection into a successful partial report.

If the product later needs to report from existing facts without refreshing every configured
resource, that should be a separate explicit mode with its own selection and freshness contract. It
must not be inferred from an absent collector branch.

## Architecture and ownership

The proposed ownership remains aligned with the repository taxonomy:

| Concern | Owner |
|---|---|
| Portfolio, network, wallet, symbol, and reader models | Existing portfolio model crates |
| Resource-specific observation and fact writing | Existing collector operation/state crates |
| Relational requirement derivation and graph composition | `mfm-op-portfolio-collect-report` |
| Generic readiness proof | Shared state or composition-owned pure state, depending on reuse |
| Report graph | Existing `mfm-op-portfolio-tracker` and portfolio states |
| Runtime endpoints, credentials, and signer routing | Runtime config and adapters |
| CLI/REST parsing and rendering | Thin binaries |

The catalog remains an authoring and pre-planning source. It stores exact typed semantic values, not
an opaque dynamic collection plan. The plan is derived after exact catalog resolution and becomes
part of the certified execution graph.

## Determinism, certification, and replay

The same validated portfolio, policy, and supported collector descriptors must produce the same
requirement ordering, child scopes, operation identities, and readiness expectations.

The composed graph must continue to satisfy these invariants:

- no ambient IO during planning;
- no secrets in config, events, facts, artifacts, or public output;
- collector and report operation descriptors are included in certification;
- child lineage and requirement identities are retained as ordinary typed evidence;
- live execution binds runtime capabilities only after admission;
- replay recomputes pure planning/readiness/report logic from the admitted spec and retained
  evidence, without loading the catalog or runtime config.

## Migration from the current composition

The existing `mfm.portfolio/collect_then_report@1` workflow is the first implementation slice. Its
current BTC/EVM native branches establish the parent-child operation lineage, typed readiness, and
report gating needed by this RFC.

The generic evolution should proceed by:

1. defining the requirement model and explicit unsupported-resource semantics;
2. moving native BTC/EVM derivation behind that model without changing collector semantics;
3. changing readiness from family counts to exact requirement identities;
4. adding token/protocol collector mappings only when their typed collectors and fact-selection
   contracts exist;
5. preserving the standalone collector entry points and report-only `portfolio_snapshot` entry
   point.

No caller should need to know whether a portfolio is EVM-only, Bitcoin-only, native-only, or a
multi-resource configuration.

## Open questions

- Which `BalanceReaderConfig` and protocol-position kinds are supported in the first generic
  version?
- Does every configured reader imply a required refresh, or should the semantic request include an
  explicit collect/reuse policy?
- What is the exact batching key for token and protocol collectors that require one shared source
  anchor?
- Is an empty portfolio always an error, or should an explicit empty-report mode be introduced?
- Which requirement identity and coverage evidence should be exposed in the public report versus
  retained only as run evidence?

## Acceptance criteria

The generic operation is ready for implementation when:

- an EVM-only portfolio expands no Bitcoin collector nodes;
- a Bitcoin-only portfolio expands no EVM collector nodes;
- a native-only portfolio expands no token or protocol collector nodes;
- every configured supported reader expands at least one matching typed collector branch;
- every configured unsupported reader fails before report execution;
- readiness detects missing, duplicate, unexpected, empty, or unacceptable child results;
- the report operation cannot execute after partial collection;
- the operation remains deterministic and certifiable;
- standalone collectors and report-only portfolio snapshots remain independently usable;
- CLI and REST entry points use the same composed planning semantics;
- focused tests cover each configuration matrix case and evidence-only replay.

## Related documents

- `RFC_CONFIG.md` — catalog-backed semantic configuration and operation composition boundaries
- `docs/portfolio-collect-then-report.md` — current fact, anchor, and report authority contract
- `docs/architecture.md` — operation/state/adapter/transport placement rules
- `docs/design.md` — certification, persistence, replay, and secret-boundary invariants
- `crates/ops/portfolio-collect-report-op/src/lib.rs` — current composed implementation slice
