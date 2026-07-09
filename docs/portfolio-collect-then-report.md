# Portfolio collect-then-report recipe

Cutover authority model: collectors write Platform data facts; `portfolio_snapshot`
is **report-only** (SelectHoldings over admitted facts). There is **no** dual live
path and **no** mixed certified draft that expands collectors inside the report op.

## Authority split

| Surface | Role |
|---|---|
| `btc_address_balance` / `evm_native_balance` | Observe chain (joint tip + pin-in balances) → record Platform facts |
| `btc_chain_head_collector_cycle` | Control checkpoint / chain-head observation only — **not** report pin authority |
| `portfolio_snapshot` | Resolve subjects → SelectHoldings → valuations → assemble → project report |

Forbidden at cutover:

- Mixed certified draft: collectors + report in one `portfolio_snapshot` expand
- Report constructing live BTC/EVM balance providers
- Collectors Platform-querying prior portfolio holdings for observe
- Optional shared tip for multi-subject same-network batches (shared tip is **required**)

## External multi-run recipe (dual mainnet natives)

Operators run collectors, then report, as separate certified runs:

1. **Collect BTC** (same-network multi-address batch shares one best-tip joint tip):

   ```text
   run btc_address_balance
     network = bitcoin-mainnet
     addresses = [ ... configured BTC addresses ... ]
   ```

   Graph: `resolve_joint_tip` at the best tip once → per address `observe@hash` → `record` → assemble batch.

2. **Collect EVM** (same chain multi-account batch shares one joint tip):

   ```text
   run evm_native_balance
     network = ethereum-mainnet
     chain_id = 1
     accounts = [ ... lowercase 0x-prefixed configured accounts ... ]
   ```

   Graph: `resolve_joint_tip` (latest block hash) once → per account balance at
   that block hash → re-verify the canonical block → `record` → assemble batch.
   Each account observation consumes exactly two source reads: the pinned block
   lookup and the balance lookup.

3. **Report** (facts only; no live chain reads):

   ```text
   run portfolio_snapshot
     portfolio config = dual-mainnet natives
   ```

   Selection is network-coherent over Platform facts under fixed policy
   `mfm.portfolio.holding.latest-network-coherent.v1`. Pins are projected from
   selected fact anchors. Each public observation carries selected holding
   `coverage` (`configured_only` or `complete_at_anchor`).

Repeat collect runs whenever configured subjects need fresh anchors; then re-run report.

## Prove-before-write (collectors)

Before any Platform write, collectors fail closed when:

- height/hash missing
- balance not proven at the joint tip hash (BTC pin-in request; EVM hash selector + tip re-verify)
- tip drift / hash mismatch
- coverage not in `{configured_only, complete_at_anchor}`
- `source_status != ok`
- secrets / `wallet_id` / `symbol_id` would appear on fact subject/response

Multi-subject same-network batches **must** share one joint tip resolved once so
all written facts carry the same anchor (F26).

## Report never live-reads

`portfolio_snapshot` and portfolio adapters use fact-index reads only. They do not
construct live BTC/EVM chain providers for balances. Missing facts fail closed
(`missing_fact` / `no_common_network_anchor`); there is no soft-success partial report.

## Operational note on tip races

If multi-subject same-network collects run without a shared in-batch tip (for
example separate single-address runs at different tips), network-coherent
selection may fall back to an older common anchor or fail with
`no_common_network_anchor`. Prefer one multi-address/multi-account batch per
network so the newest tip is common.

## Design contract

Normative authority model and cutover rules: [`RFC_COLLECTORS_PORTFOLIO.md`](./RFC_COLLECTORS_PORTFOLIO.md).
