# Request-author Totality Audit

Contract id: `mfm.request-author-totality-audit.v1`

This is retained pre-cutover evidence for the
[`mfm.portfolio/snapshot@1`](recoverability-cutover-gates-v1.md#published-entry-points-and-planning-profiles)
schema-freeze gate. It does not change the current runtime, registry, admission API, or authoritative
design. The eventual vertical cutover must delete the aggregate readers and carry forward only the
qualified shapes recorded here.

## Closed production-state inventory

The published snapshot catalog has five reachable production state types and no effect state.
Branch-union conformance covers Bitcoin-only and EVM-only accepted portfolio configurations, so a
declared but unreachable catalog entry cannot silently satisfy this inventory.

| Current state | Current request shape | Cutover disposition |
| --- | --- | --- |
| `mfm.portfolio.assemble_snapshot` | Pure; authors no external request. | Retain as pure work. |
| `mfm.portfolio.project_report` | Pure; authors no external request. | Retain as pure work. |
| `mfm.evm.collect_balances` | One fallible aggregate plan expands below the audit boundary into bootstrap, anchor, metadata, balance, and confirmation RPCs. | Delete and replace with the decomposed EVM prototype: source/chain bootstrap, initial anchor, one metadata or balance state per RPC, final confirmation, and pure aggregation. |
| `mfm.bitcoin.collect_balances` | One fallible aggregate plan performs bootstrap, `scantxoutset "start"`, and confirmation RPCs. | Delete the aggregate state. Keep Bitcoin collection unregistered until the indivisible scan passes its separate repeat-work and provider-cost qualification. |
| `mfm.portfolio.select_holdings` | Receipt validation and query construction can fail inside `ReadState::plan`; one accepted plan authors a variable number of fact queries. | Move receipt/config validation and query compilation into upstream pure work that emits one checked frame per fact selection. Each read state then authors exactly one `FactSelectionRequest`; aggregation is pure. |

`ValidateEvmContractState` and `SubmitEvmTransactionState` are exported library types but are not
in the snapshot authoring catalog and are not reachable from the published entry point. They do
not enter this gate. EVM mutation remains unregistered in the core cutover.

## Totality evidence

An accepted request input is the stronger, privately constructed frame presented to a read/effect
request callback, not every structurally deserializable predecessor value. Fallible semantic
validation occurs before that frame exists. The callback has the compile-time shape
`fn(&CheckedFrame) -> Request`, returns exactly one request value, and is exercised twice for
determinism.

- `crates/domains/evm/src/operation_tests/recoverability_prototype.rs` retains the decomposed EVM
  graph and infallible bootstrap, latest-anchor, metadata, balance, and confirmation author
  signatures. Its bounded maximum has 2,051 independently audited external operations and one pure
  aggregation node.
- `crates/domains/bitcoin/src/operation_tests.rs` retains the analogous three-request chain and
  infallible author signatures. Transport conformance deliberately leaves the scan capability
  unregistered because total authorship does not qualify repeated provider work.
- `crates/domains/portfolio/tests/request_author_totality.rs` closes the catalog inventory,
  exhaustively varies small accepted aggregate EVM shapes, exercises every supported Bitcoin
  network tag, proves current aggregates contain multiple application operations, and verifies
  one-request checked fact-selection frames across one through eight holdings. A missing receipt
  produces no checked frame.

The current fallible `ReadState::plan` signature is not grandfathered. These tests identify the
validation/decomposition boundary to carry into the new closed `StateExecution` callback contract;
they do not certify the current aggregate states as schema-freeze-ready.

## Exact entry-point/profile fixture

The same portfolio conformance target retains a private prototype catalog with exactly one
admission mapping:

```text
mfm.portfolio/snapshot@1
  -> stable entry_point_operation_id: mfm.portfolio/snapshot
  -> root operation descriptor name: mfm.portfolio.snapshot
  -> retained canonical authored-program bytes
  -> mfm.planning-profile.v1
       framework_policy_refs: []
       planner_contract_ref: mfm.composite-planner.v1
       planner_implementation_ref: content address of the retained prototype implementation
       canonical_profile_parameters: {}
  -> expanded composite-graph hash and prototype certificate hash
  -> private sealed admission authority
```

The entry-point identities match the
[`mfm.recoverability-app-surface.v1`](recoverability-app-surface-v1.md#published-entry-point-and-planning-profile)
contract: the versioned start id, stable slash-form operation id, and dot-form Rust operation name
cannot substitute for one another. The empty ordered framework-policy list and empty canonical
parameter object are required fields, not defaults. Verification parses the retained authored bytes
again, independently reruns the private profile-aware composite graph encoding, and requires
byte-identical expanded bytes plus a certificate bound to their hash. Unknown entry-point ids,
changed profiles, omitted policy lists, cross-substituted identities, raw library drafts, and raw
library certificates cannot mint the private admission authority. This fixture remains
conformance-only until the schema freeze defines the production `PlanningProfile`, retained
authored-program artifact, certificate fields, and sealed app admission boundary.

Every retained prototype reference and hash derives only as
`SHA256(JCS({"domain": <prototype-local domain>, "value": <typed value>}))`. Raw-value, bare-prefix,
NUL-prefix, and wrong-domain hashes differ and cannot satisfy the retained certificate. Strict
envelope parsing rejects noncanonical bytes, floats, and duplicate keys. These prototype-local
domain tags do not define the production domain registry; the canonical schema annex owns that
registry and its shared vectors.

## Material uncertainties

none
