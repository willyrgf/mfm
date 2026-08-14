# EVM and Portfolio public-contract freeze

Status: binding implementation contract for the EVM/Portfolio completion
cutover.

## Material uncertainties

none. This document resolves the three uncertainties recorded in
[`FINISH_EVM_PORTF_REFACTOR.md`](../FINISH_EVM_PORTF_REFACTOR.md) against the
last production-shaped baseline,
`b223ae17e07844d59df05e61ea253b1dedfb90c6`.

## Frozen public results

The canonical JSON fixtures in
[`docs/contracts/evm-portfolio`](contracts/evm-portfolio) are the source of
truth for the public projections. They are RFC 8785/JCS-style canonical JSON:
objects have lexicographically ordered keys, strings and decimal quantities
are already normalized, and no fixture contains a floating-point value.

### EVM submission

`EvmSubmissionOutput` is deliberately redacted. Its entire public success
projection is one `execution_disposition` field:

```json
{"execution_disposition":"succeeded"}
```

or:

```json
{"execution_disposition":"reverted"}
```

Candidate ids, transaction hashes, provider responses, signatures, nonce
details, route identities, and all receipt/finality evidence remain retained
domain evidence. They are not public output fields.

The `reverted` projection is still a successful terminal result: it is an
authenticated, canonical execution observation, not a transport failure.
Failure projections are exactly these payload-free `{"kind":"..."}` values:
`nonce_authority_unavailable`, `destination_rejected`, `provider_unavailable`,
and `nonce_lineage_diverged`. They never expose a provider diagnostic,
candidate, hash, nonce, signer, or binding identity. Selector or admission
rejection occurs before a submission run exists and is not serialized as one of
these terminal failures.

### EVM balance collection

The collection completion is an internal, typed domain handoff consumed by
Portfolio. It nevertheless has a frozen canonical shape because it crosses
the EVM/Portfolio ownership boundary. Each collection contains exactly once:

- the caller-owned `collection_ordinal` and bounded `caller_context`;
- the EVM-owned checked chain and common anchor;
- source results in admitted declaration order, each with its source identity,
  exact raw integer quantity, and decimal scale; and
- the checked integer-scaled collection total.

The ordinal and correlation are not inferred by Portfolio and must not be
dropped, duplicated, or replaced with a runtime occurrence address. The
fixture records the required shape without importing the retired fan-out or
wallet-lifecycle model.

### Portfolio snapshot

The public Portfolio result remains a `snapshot` plus a deterministic `report`
projection. The snapshot owns the selected portfolio identity, checked network
pins, declaration-ordered holdings, and the public source metadata required to
interpret each amount. The report is derived from that exact snapshot and owns
the selected quote, declaration-ordered per-collection summaries, and checked
aggregate totals. `collection_totals` alone is not a sufficient public result.

The completion cutover may use smaller current domain types internally, but it
must reproduce the frozen projection semantics and canonical bytes. It must
not restore the retired fan-out workflow, configuration/route DTOs, wallet
lifecycle, certification machinery, or any provider topology.

## Field ownership and evidence

| Projection / field | Owner | Exact source | Validation before projection |
| --- | --- | --- | --- |
| `EvmSubmissionOutput.execution_disposition` | EVM submission consolidation | bound receipt status, receipt block, finalized head, and canonical inclusion-block evidence | one committed broadcast intent; transaction hash correlation; receipt/inclusion block hash and number agreement; receipt chain/target binding; finalized head covers the inclusion block |
| EVM submission failure `kind` | EVM State | declared State failure or fixed integrity failure | one of the four frozen payload-free discriminants; no diagnostic or correlation payload |
| collection ordinal/correlation | Portfolio planner | admitted `PortfolioSnapshotInput` | dense declaration position and bounded caller correlation agree with the admitted demand |
| checked chain / common anchor | EVM balance fragment | exact bound chain-identity, initial-anchor, and confirm-anchor Reads | every response is intent-bound; chain and anchors agree with the selected binding and source sequence |
| source raw quantity / decimals | EVM balance fragment | exact bound native/token balance and token-decimals Reads | asset kind, source, anchor, decimal bounds, canonical integer quantity, and overflow checks agree |
| collection total | EVM balance consolidation | completed source results | exact source order/uniqueness; common anchor; checked integer-scaled addition |
| Portfolio collection failure `code` | EVM balance failure mapped by Portfolio | EVM-owned closed redacted stage code | check-chain rejection is `chain_identity_unavailable` (the frozen golden); later unavailable observations, invalid collection consolidation, and integrity blocks use their distinct reviewed codes, never provider text |
| Portfolio snapshot/report fields | Portfolio consolidation | admitted Portfolio input and exact EVM collection completions | collection order, route/binding agreement, source realization, checked decimal arithmetic, and selected quote agreement |

## Submission program suffix

A broadcast acknowledgement is not inclusion evidence. The final EVM
submission Program is therefore fixed to this post-broadcast suffix:

```text
broadcast                         one-entry Effect
receipt                           Read
finalized head                    Read
canonical inclusion block         Read
disposition consolidation         Pure -> EvmSubmissionOutput
```

The receipt Read is bound to the transaction hash returned by the committed broadcast call. The
canonical-inclusion Read is bound to the receipt block number and must return its exact hash.
Consolidation accepts only a receipt whose transaction hash and block agree with those two Reads,
whose status maps to `succeeded` or `reverted`, and whose inclusion block is covered by the
authenticated finalized head. Any unavailable, rejected, malformed, or inconsistent evidence takes
the declared typed failure/integrity route; it cannot manufacture either public disposition.

## Standalone binaries

The standalone CLI and REST binaries have no deployable trusted composition:
they do not own endpoint discovery, credentials, key custody, nonce authority,
or a live Runtime registry. Their actionable EVM submission and Portfolio
admission/drive commands and routes are therefore removed by the vertical
cutover. They may expose only non-actionable local help/version behavior until
a separately scoped trusted embedding supplies a real `Application`.

No fake, null, scripted, or default provider is a production substitute.
Scripted providers are test-only.
