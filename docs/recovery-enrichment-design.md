# Explicit candidate-asset enrichment

This is the dedicated architect's target for the separate enrichment commit required by
[the recovery RFC](../RFC_REFACT_RUNTIME_TO_RECOV_SM.md). Complete the execution replacement first.

## Domain and planning

The caller supplies a bounded candidate asset list. Enrichment retains every configured native
source and each candidate token with nonzero raw balance at the collection's verified anchor.
It makes no claim to find assets outside that list. Require at least one native source per
collection so publication never needs an empty-collection contract. Preserve candidate order,
source IDs, account and chain identity, quotes and route bindings.

Add a Portfolio enrichment entry point and `ResolvePortfolioAssets` Pure State. Factor the common
initialize/enter/`CollectEvmBalances`/resume authoring prefix once, then choose the final State
according to the entry point. Reuse the existing anchored provider Reads and aggregate 64-source
bound. Provider or integrity failures fail the enrichment run instead of silently excluding assets.
Dependent execution continues to read token decimals through the ordinary anchored contract.

`PortfolioEnrichmentOutput` contains the resolved checked Portfolio configuration, exact public
chain/route bindings and bounded collection anchors. Domain derives it from the retained original
candidate demand and checked collection results. Application does not inspect balances to select
assets. The output contains no self-referential RunId/head/output linkage.

## Publication and admission

`Application::publish_enrichment(destination_name, enrichment_run_id)` reads the run, requires
successful completion with the exact output schema, validates resolved values and pre-bound routes,
and constructs a canonical snapshot config with provenance: enrichment RunId, terminal head and
exact output reference. Convert route references through the immutable composition's public
binding inventory; reject missing/inconsistent bindings and never resolve arbitrary new endpoints.
Use the existing opaque `ConfigRepository::import_config` and its Created/Unchanged result.
Publication accepts no replacement discovery request or resolved values. Repetition, including
a lost acknowledgement, reads the same output and imports the same revision without discovery IO.

Static snapshot documents remain valid without enrichment provenance. Before new dependent
admission, Application verifies enriched provenance against the successful run's exact head,
output reference/schema, resolved configuration and bindings. Imported provenance is not trusted
merely because JSON contains it. The dependent initial value retains the full resolved demand,
enrichment provenance and a bounded source-revision identity sufficient to compare ConfigSelection;
the domain gains no ConfigRepository dependency.

Matching start recovery reads the requested dependent RunId before loading configuration. Expose
the qualified genesis input through the existing RunView as a ValueView accessor. For a retained
run, decode its admission identity and require the same selection/entry point; otherwise return a
stable conflict. For an absent run, load the selected config and validate its linkage before start.
An unavailable/uncertain read stops admission. Normal read/progress after admission never reloads
configuration or enrichment history. Config deletion is not revocation.

Use the existing config/start/read/progress use cases for enrichment, adding only explicit
publication and matching CLI/REST surfaces. The caller supplies a separate RunId for the dependent
snapshot. A private typed planner enum may distinguish entry points. Add no erased public value
system, automatic publication/admission chain, expiration, rediscovery or background job.

## Commit and verification

Keep the complete current execution replacement as one coherent commit. Then implement enrichment
as one coherent commit covering domain, Application, transports, tests and documentation. Final CI
runs on the candidate containing both.

Verify live loopback candidate selection/order/native retention, zero token exclusion and provider
failure; exact schema/route/provenance rejection; maximum candidate/output/config bounds; ordinary
recovery; Created/Unchanged and lost-ack publication without discovery; a new immutable revision
from a new enrichment run without changing the old revision/history; dependent admission;
configuration deletion followed by progress and matching start recovery; conflicting selection
rejection; time-independent publication/admission; and managed PostgreSQL plus CLI/REST contracts.
Publish after deleting the candidate configuration to prove retained input is sufficient.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| The current continuation retains sufficient original demand. | Publication does not exist yet. | Add only missing immutable planning fields. | Publish after deleting the candidate config. |
| Resolved output and provenance fit the 256 KiB config bound. | The new document shape is not measured. | Tighten checked enrichment bounds explicitly. | Maximum 64-source candidate/output/publication fixtures. |
| RunView can expose genesis input through its current qualification boundary. | The accessor is absent. | Extend existing view construction without another folding/inspection path. | Matching/conflicting start recovery after deletion and uncertain admission. |
