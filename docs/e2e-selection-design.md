# E2E design: select an existing operation

This is the proposed target for scenario 1 of the
[public interfaces RFC](../RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md), not a description of
implemented convenience APIs. Names marked proposed require a reviewed contract before coding.
The [design](design.md) and [architecture](architecture.md) retain authority over execution,
error preservation, secrets, and persistence.

## Objective and story

A caller discovers, selects, configures, executes, and observes maintained MFM behavior through
either CLI or REST without authoring an Operation, State, registration table, or execution loop.
Both transports must demonstrate independent execution over the same Application surface.

The caller requests a Portfolio snapshot containing one funded and one empty native account.
This is useful behavior with explicit inputs, observable external evidence, a meaningful report,
and an independent oracle. It isolates selecting an operation from composing or extending it.
The choice does not depend on retaining today's API, and does not make transaction submission or
general-chain finality a prerequisite for demonstrating selection.

## Fixture and caller choices

Managed setup supplies pinned Reth, a fresh split-role PostgreSQL base run/configuration schema,
built CLI and REST binaries, and temporary deployment/configuration/socket files. No signer,
keystore, transaction-authority schema, or Solidity compiler is needed for this scenario.

Use these public inputs:

| Input | Fixture value |
| --- | --- |
| Entry point | `mfm.portfolio/snapshot@1` |
| Config name / portfolio | `public-selection` |
| Quote | `usd` |
| Chain / public endpoint | `1337` / `reth-dev` |
| Collection | `native-accounts` |
| Funded source | `funded`, address `0x70997970c51812dc3a010c7d01b50e0d17dc79c8` |
| Empty source | `empty`, address `0x0000000000000000000000000000000000000001` |
| Assets / decimals | native assets, 18 |
| Run identities | Two distinct explicit valid RunIds, selected before execution |
| Invocation policy | A bounded 30-second wait budget |

The pinned genesis funds the first account with `1000000000000000000000000` raw units
and leaves the second at zero. Fixture setup must verify those premises independently; it must
not use an MFM snapshot to obtain expected values. Keep the node isolated from concurrent writes.
Deployment resolves Store/provider locators privately. The operation request contains only public
binding selectors, never credentials, locator values, or runtime implementation registrations.

An illustrative complete config uses the current document vocabulary:

```json
{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {
    "routes": [{"chain_id": 1337, "endpoint_id": "reth-dev"}],
    "selector": {"target": "public-selection", "quote": "usd"},
    "portfolio": {
      "portfolio_id": "public-selection",
      "quotes": ["usd"],
      "collections": [{
        "correlation": "native-accounts",
        "request": {
          "sources": [
            {"source_id": "funded", "chain_id": 1337,
             "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null},
            {"source_id": "empty", "chain_id": 1337,
             "address": "0x0000000000000000000000000000000000000001", "token": null}
          ],
          "decimals": 18
        }
      }]
    }
  }
}
```

This example does not freeze redundant configuration fields. Before implementation, check that
each retained field represents a caller decision rather than repeated assembly knowledge.

## Proposed public calls

CLI discovery and selection use existing command concepts; `--wait` is proposed:

```text
mfm_cli --output json entry-point list
mfm_cli --deployment deployment.toml --output json binding list
mfm_cli --deployment deployment.toml --output json config import public-selection --from snapshot.json
mfm_cli --deployment deployment.toml --output json run start \
  --config public-selection --config-digest <digest> --run-id <cli-run-id> --wait 30s
mfm_cli --deployment deployment.toml --output json run show --run-id <cli-run-id>
```

The REST client uses the real Unix-socket listener:

```text
GET /v1/entry-points
GET /v1/bindings
PUT /v1/configs/public-selection
POST /v1/runs/start
GET /v1/runs/<rest-run-id>
```

PUT carries the same complete config. The proposed start request adds an invocation wait budget:

```json
{
  "config": {"name": "public-selection", "digest": "<digest>"},
  "run_id": "<rest-run-id>",
  "execution": {"wait_ms": 30000}
}
```

Placeholders stand for checked identities returned by import or selected by the fixture.
Successful start returns the selected config summary and shared RunView. Success includes exact
output contract/value references and the inline Portfolio value. CLI exits 0; REST returns 200.
Cold show returns the same terminal view. Preserve the documented transport asymmetry for a
durably failed run: CLI exit 1, REST 200 with a tagged failed state.

### Common execution and stopped outcomes

Application delegates ordinary waiting to the proposed `RuntimeDriver`, the same maintained
Runtime driver used directly by scenarios 2 and 3. Binaries still perform one Application call.
The test must not implement its own poll/progress/retry loop. Process supervision timeouts remain
test infrastructure and are distinct from the requested execution budget.

The wait budget is invocation policy, not persisted recovery authorization. It does not change
Program identity, recovery allowances, command identity, or the admitted input. The driver may
continue only progression authorized by Runtime; it cannot turn an unresolved outcome into a
replacement command or silently start with a fresh RunId.

Expiry ends the invocation, not the run, and returns an explicit stopped-invocation outcome with
its RunId, available cause, and last qualified observation. Cancellation or client/process loss may
leave no response; recovery uses the retained identity without a promised cancellation record.
An observation is historical;
only explicit acknowledgement evidence is labelled acknowledged. Do not return a fabricated
terminal failure or claim an interrupted physical attempt was recorded. The exact timeout wire
code and HTTP status remain a contract decision; resolve them before implementing `--wait`.

`RecoveryStopped`, Store/invocation failures, and indeterminate appends stop automatic driving.
The driver must not repeatedly resume past a stopped recovery decision. Indeterminate append
retains exact start/progress recovery identity and causal facts.
Any subsequent recovery is explicit and uses the same identity. A proposed bounded wait option on
`run progress` / the progress request delegates to the same driver. Deliberate single-invocation
progress remains available for boundary cases; it is not the baseline consumer workflow.

## Baseline run and independent assertions

1. Start isolated infrastructure and discover the entry point and public binding through each
   transport. Import through one client and repeat through the other: `created`, then `unchanged`,
   with the same exact revision.
2. Execute independently through REST and CLI with distinct explicit RunIds and the wait budget.
   Assert selected name/digest, returned RunId, successful terminal phase, exact output contract,
   and qualified value reference. Decode through public client/output contracts.
3. Independently assert source order, native asset identity, decimals, raw balances, and amounts
   `1000000.000000000000000000` and `0.000000000000000000`. Assert the current fixture's declared
   Portfolio report rule yields collection and USD totals `1000000`; this is not a market-price
   assertion. Expected values are literals from fixture premises, not production calculations.
4. Query fixture RPC independently for the reported block number/hash and balances at that exact
   anchor. Require those facts to agree with the output. Do not validate only that an anchor has
   the right JSON shape, or only that both MFM transports agree.
5. Compare semantic outputs across independent executions. Require full equality only if fixture
   scheduling guarantees the same anchor; otherwise validate each anchor independently.
6. Restart the listener and observe each run through the other transport. Require unchanged head,
   value reference, and complete output. Repeated terminal progress must not append.

The baseline does not freeze internal State counts or terminal frame numbers. Exact intermediate
heads belong in cases whose purpose is durability. Checked decoding does not replace independent
wire assertions for transport behavior or an independent business oracle.

## Named cases and coverage ownership

Each case is an independently diagnosable run within this public-usage scenario. Cases need not
form one long stateful test. Future additions cannot justify removing existing guarantees.

| Case | Status | Evidence / existing owner |
| --- | --- | --- |
| `select_snapshot_through_each_transport` | Preserved and strengthened | Independent CLI/REST runs, complete result and independent anchor oracle, cold equality; current client E2E main test. |
| `generated_identity_remains_recoverable` | Preserved | Generated REST identity remains discoverable after interrupted response; independent generated CLI identity and result; current main test. |
| `admitted_selection_survives_revision_deletion` | Preserved | Block first live Read, prove durable runnable prefix, retain another same-name revision, delete selected revision, cold-resume original admitted selection; current main test. |
| `provider_failure_survives_cold_client_observation` | Preserved | Supplied RPC code/message/data and causes survive durable failure and fresh REST/CLI observation with their respective status/exit semantics; current main test. |
| `enrichment_publication_preserves_provenance` | Preserved | Enrich, delete source config, publish through REST, repeat through CLI unchanged without discovery, execute dependent snapshot, delete published config, recover exact start; current `verify_enrichment_publication`. |
| `start_and_progress_preserve_recovery_envelopes` | Preserved at focused owners | Existing frozen renderer fixtures prove exact identity/envelope equivalence, not real ambiguous COMMIT injection. |
| `wait_budget_stops_and_same_run_resumes` | Required new coverage | Short budget produces honest stopped outcome, no invented failure; explicit progress resumes same RunId. Required when adding waiting, not deferred. |
| `append_acknowledgement_loss_across_clients` | Future strengthening | Real fault plus same-identity recovery; preserve any existing focused owner meanwhile. |
| `concurrent_matching_start` | Future strengthening | Concurrent callers cannot create different work under one identity. |

The current source is [client_execution_e2e.rs](../bin/rest-api/tests/client_execution_e2e.rs).
Recovery rendering fixtures live under [client-surface](contracts/client-surface/).
Existing focused bounds, entropy, output-failure, capacity, and conflicting-selection tests retain
their owners unless explicitly reassigned. No need to repeat every boundary through both clients.

## Ownership and complete cutover

Test-owned work is scenario inputs, independent expected values, processes/services, temporary
files, transport requests, deliberate RPC block/reject fixtures, and assertions. Fault fixtures
are not production operation options. Production owns discovery, validation/planning, exact
revision handling, association/binding, ordinary driving, checked result preparation, and cold
interpretation. The test defines no custom Operation or State.

The existing client test already largely respects these responsibilities. Reshape its monolithic
main test and publication helper into named cases sharing infrastructure, not a scenario DSL.
Delete superseded orchestration and ordinary driver duplication in the same cutover. Preserve
legitimate process/socket/fault helpers. Do not freeze raw JSON traversal where checked public
models can express the assertion, or move expected answers into production.

1. Settle the common driver action/outcome and timeout contracts with the library scenarios.
   Implement shared behavior with focused Runtime/Application/client tests and update relevant
   design, architecture, and transport documentation in the same logical change.
2. Replace the old client E2E organization with these named cases and explicit coverage mapping.
   Keep existing focused recovery renderers. Update both binary READMEs and managed coverage
   documentation; delete superseded test orchestration with its replacement.
3. Run affected focused tests and managed `nix run .#run -- --task client-e2e`. Retain that task
   identity unless the complete three-scenario cutover has a concrete reason to rename it.
   Follow [build and verification](build-and-verification.md) for final CI after executable
   cross-crate/task changes. This design document itself needs links and whitespace review only.

## Material uncertainties

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| One bounded wait option serves all three usage paths. | The common driver and timeout code/status are not yet selected. | Divergent drivers or misleading timeout authority. | Agree one action/outcome matrix and timeout wire contract; test ambiguity, failure, cancellation, and same-identity resume before shipping. |
| Fixture executions can share one anchor. | Node mining or concurrent activity may advance it. | Incorrect full-equality assertions become flaky. | Isolate fixture activity or validate each reported anchor and compare semantic holdings. |
| Every retained configuration field represents a caller choice. | Current request repeats route/identity information. | Freezing it perpetuates assembly burden. | Review the smallest complete request and justify each retained field before implementation. |
| Splitting existing cases retains process-level recovery evidence. | Their current order combines blocking, listing, deletion, and restart. | Simplification could stop proving cold recovery after response loss. | Map every retained assertion to a named case and preserve durable prefix, identity lookup, and deleted-revision evidence. |
