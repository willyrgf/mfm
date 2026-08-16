# EVM Read and Portfolio public-contract freeze

Status: binding implementation contract for the surviving Portfolio product surface.

## Material uncertainties

none.

## Frozen public results

The three surviving canonical JSON fixtures in
[`docs/contracts/evm-portfolio`](contracts/evm-portfolio) are the source of truth for public and
domain-handoff projections. They are RFC 8785/JCS-style canonical JSON: objects have
lexicographically ordered keys, strings and decimal quantities are normalized, and no fixture
contains a floating-point value.

### EVM balance collection

The collection completion is an internal, typed domain handoff consumed by Portfolio. It has a
frozen canonical shape because it crosses the EVM/Portfolio ownership boundary. Each collection
contains exactly once:

- the caller-owned `collection_ordinal` and bounded `caller_context`;
- the EVM-owned checked chain and common anchor;
- source results in admitted declaration order, each with its source identity, exact raw integer
  quantity, and decimal scale; and
- the checked integer-scaled collection total.

The ordinal and correlation are not inferred by Portfolio and must not be dropped, duplicated, or
replaced with a Runtime occurrence address.

### Portfolio snapshot

The public Portfolio result is a `snapshot` plus a deterministic `report` projection. The snapshot
owns the selected portfolio identity, checked network pins, declaration-ordered holdings, and the
public source metadata required to interpret each amount. The report is derived from that exact
snapshot and owns the selected quote, declaration-ordered per-collection summaries, and checked
aggregate totals. `collection_totals` alone is not a sufficient public result.

## Field ownership and evidence

| Projection / field | Owner | Exact source | Validation before projection |
| --- | --- | --- | --- |
| collection ordinal/correlation | Portfolio planner | admitted `PortfolioSnapshotInput` | dense declaration position and bounded caller correlation agree with the admitted demand |
| checked chain / common anchor | EVM balance fragment | exact bound chain-identity, initial-anchor, and confirm-anchor Reads | every response is intent-bound; chain and anchors agree with the selected binding and source sequence |
| source raw quantity / decimals | EVM balance fragment | exact bound native/token balance and token-decimals Reads | asset kind, source, anchor, decimal bounds, canonical integer quantity, and overflow checks agree |
| Portfolio route identity | Portfolio C0 carried by the EVM balance context | exact planned Read intent and bound EVM adapter descriptor | each balance Read intent retains the selected secret-free physical-target ref; the adapter compares it with the committed descriptor before provider entry |
| collection total | EVM balance consolidation | completed source results | exact source order/uniqueness; common anchor; checked integer-scaled addition |
| Portfolio collection failure `code` | EVM balance failure mapped by Portfolio | EVM-owned closed redacted stage code | unavailable observations, invalid collection consolidation, and integrity blocks use distinct reviewed codes and never provider text |
| Portfolio snapshot/report fields | Portfolio consolidation | admitted Portfolio input and exact EVM collection completions | collection order, route/binding agreement, source realization, checked decimal arithmetic, and selected quote agreement |

## Unsupported transaction submission

The product has no EVM transaction-submission entry point, planner, State graph, capability,
signer/nonce/broadcast adapter composition, or result fixture. Reintroducing submission requires a
future RFC that defines durable transaction authority and outbox semantics; a disabled route,
in-process nonce reservation, or direct provider broadcast is not an acceptable substitute.

## Standalone binaries

The standalone CLI and REST binaries have no deployable trusted composition. They do not own
endpoint discovery, credentials, or a live Runtime registry and therefore expose no actionable
Portfolio admission/drive surface. No fake, null, scripted, or default provider is a production
substitute; scripted providers are test-only.
