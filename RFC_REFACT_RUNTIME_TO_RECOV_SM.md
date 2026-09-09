# RFC: recoverable linear execution

Status: implemented. [Design](docs/design.md) owns the current contracts;
[architecture](docs/architecture.md) owns responsibility and placement;
[validation](docs/recovery-validation.md) records candidate-specific evidence.
This RFC retains the decisions and rationale, not a second API specification.

## Decision and rationale

An admitted Program is one immutable, content-addressed sequence of typed States. Normal success
advances linearly. Runtime owns recovery transitions rather than requiring authors to duplicate
business sequences or encode bounded retries as forward graph branches.

Planning uses the complete checked ordered source plan. Native/token selection is known before
admission and does not require a runtime Match. Root input validation and exact initial-value
commitment prevent execution from substituting values inconsistent with those planning assumptions.
An admitted Program cannot grow or change; IO needed to determine a future plan belongs to a
separate explicit enrichment run and immutable configuration revision.

Classifier and handler selection remain independent. The classifier assesses original domain
failures or typed adapter errors with State context; the handler requests recovery. Explicit maps
adapt domain failures and contexts, while original operational causes remain unchanged. Families
inherit through authoring scopes, and occurrence overrides resolve to exact finite bindings before
admission. Runtime receives the selected descriptors, not a policy-inheritance interpreter.

Runtime independently authorizes each request against mode, eligible checkpoints, local/global
allowances, and Effect barriers. Checkpoints retain the active input at an authored boundary;
restart restores it without resetting spent allowances. Scoped typed tokens prevent foreign or
escaped checkpoint installation. Execution visits remain distinct from budget counters.

A recovery decision and its incident commit in one conclusion frame. Cold reconstruction validates
and applies the recorded decision without calling policy implementations again. Public observations,
pre-append checks, and reconstruction use the same semantic fold. Journal establishes structural
trust in stored bytes; hash linkage alone does not establish semantic legality. Store remains an
atomic exact-head append/complete-load port with no Program or recovery knowledge.

Effect preparation retains the complete command and exact Effect identity before adapter entry.
Pending observations preserve that authority; only settlement permits the adjacent conclusion.
A retained Effect bars restart across its position. Local implementation failures and ambiguous
Store acknowledgements stop the invocation; they do not fabricate durable business outcomes or
silently retry an uncertain append. Terminal reports and stopped invocations are distinct public
results, including when no durable position is known.

Inline terminal reports deliberately retain original and mapped failure payloads even when equal,
so direct consumers need no history lookup to interpret them. The 32 MiB report limit is separate
from individual value and admitted Journal bounds. Combined-report overflow returns precise safe
size diagnostics before terminal append and preserves acknowledged Runnable/EffectPending state.
This accepted limitation does not claim every admitted failure can produce a terminal report.

Enrichment selects only caller-supplied candidates. It retains native assets and tokens with nonzero
anchored balances, preserving order and failing on provider/integrity errors rather than treating
missing evidence as zero. Application verifies the exact terminal output and immutable route
bindings before publishing or accepting dependent admission. Repeated publication of the same
output is idempotent. Different output identity changes provenance and the published revision;
there is no automatic rediscovery, expiry, or rewriting of retained revisions.

## Alternatives rejected

| Alternative | Reason |
| --- | --- |
| Keep Match alongside recovery | Adds a second control model without a demonstrated runtime-branching requirement. |
| Unroll retries into a forward graph | Duplicates business work and couples limits to topology. |
| Let States return arbitrary successor IDs | Moves scheduling into domains and recreates an implicit graph. |
| Resolve authoring inheritance at execution time | Retains unnecessary scopes and lookup paths in Runtime. |
| Classify only domain failures | Excludes ordinary recoverable observational adapter errors. |
| Configure recovery for Runtime/Store faults | Adds policy execution where trusted execution or persistence cannot proceed. |
| Persist derived activations, reservations or report metadata | Duplicates authority already established by Program and committed history. |
| Reevaluate committed policies during load | Repeats decisions that have already become durable facts. |
| Treat all failures as terminal domain outcomes or generic retries | Loses uncertainty and can abandon pending Effect authority. |
| Use budget counters as execution identity or fork history on restart | Counters cannot identify visits or replace one exact-head append chain. |
| Perform discovery inside expansion | Introduces ambient IO without ordinary durable execution/recovery. |
| Append States to an admitted Program | Mutates identity and creates a second admission protocol within a run. |
| Expire enrichment or rediscover implicitly | Adds an unrequested configuration lifecycle. |
| Truncate history for rollback | Destroys acknowledged evidence and cannot undo external actions. |
| Preserve superseded decoders or a second Runtime | Multiplies current designs; the clean-slate cutover rejects superseded data. |

## Checked consuming examples

- [Scoped authoring and input commitment](crates/kernel/program/src/tests.rs), with
  [compile-fail authority checks](crates/kernel/program/tests/authoring_boundaries.rs).
- [Real parent/child/occurrence policy selection](crates/domains/portfolio/tests/recovery_policy.rs).
- [Cold association and mapping](crates/kernel/runtime/src/assembly/recovery/tests.rs),
  [recovery execution](crates/kernel/runtime/src/assembly/recovery/tests/execution.rs), and
  [nested checkpoint/budget behavior](crates/kernel/runtime/src/assembly/recovery/tests/execution/nested.rs).
- [Pending transaction authority and complete closure bounds](crates/live/evm/tests/support/recovery_transaction.rs).
- [Product execution](crates/app/tests/support/portfolio_contract.rs) and
  [publication/dependent admission](crates/app/tests/support/enrichment.rs).
- [Qualified wire and hostile history](crates/kernel/journal/tests/frame_contract.rs),
  [Runtime/Store boundaries](crates/kernel/runtime/tests/runtime_contract.rs), and
  [object/report size errors](crates/kernel/runtime/tests/report_capacity.rs).

Verification commands and gate selection belong to
[build and verification](docs/build-and-verification.md). No unchecked signature prototype is
maintained alongside these consuming examples.

## Material uncertainties

none for the selected architecture. Candidate-specific verification results and any outstanding
validation limits belong to [recovery validation](docs/recovery-validation.md).
