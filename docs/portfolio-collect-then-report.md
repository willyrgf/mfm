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

Operators run collectors, then report, as separate certified runs. The tracked configs are
`examples/configs/btc-address-balance.toml`, `examples/configs/evm-native-balance.toml`,
`examples/configs/portfolio-dual-mainnet.toml`, and
`examples/configs/runtime-dual-mainnet.toml`.

The following Bash sequence is the copy/pasteable operator path. It assumes `DATABASE_URL` points
at a migrated Postgres store, the RPC environment variables referenced by the runtime config are
set, `MFM_FACT_RECEIPT_SIGNING_KEY_FILE` points at the operator-only signing key, and `jq` is
installed. The signing key is never placed in a tracked config.

```bash
export MFM_RUNTIME_CONFIG_FILE="$PWD/examples/configs/runtime-dual-mainnet.toml"
export MFM_FACT_RECEIPT_SIGNING_KEY_FILE=/run/mfm/fact-receipt-signing-key

mfm_cli --output-format json facts provision-authority --database-url "$DATABASE_URL"

btc_response="$(mfm_cli --output-format json run start \
    --op btc_address_balance \
    --config examples/configs/btc-address-balance.toml \
    --runtime-config "$MFM_RUNTIME_CONFIG_FILE" \
    --database-url "$DATABASE_URL")"
btc_run_id="$(printf '%s\n' "$btc_response" | jq -er '.data.run.run_id')"
printf '%s\n' "$btc_response" | jq -e '.data.run.run_mode == "completed"'

evm_response="$(mfm_cli --output-format json run start \
    --op evm_native_balance \
    --config examples/configs/evm-native-balance.toml \
    --runtime-config "$MFM_RUNTIME_CONFIG_FILE" \
    --database-url "$DATABASE_URL")"
evm_run_id="$(printf '%s\n' "$evm_response" | jq -er '.data.run.run_id')"
printf '%s\n' "$evm_response" | jq -e '.data.run.run_mode == "completed"'

report_response="$(mfm_cli --output-format json run start \
    --op portfolio_snapshot \
    --op-version 2 \
    --config examples/configs/portfolio-dual-mainnet.toml \
    --database-url "$DATABASE_URL")"
report_run_id="$(printf '%s\n' "$report_response" | jq -er '.data.run.run_id')"
printf '%s\n' "$report_response" | jq -e '.data.run.run_mode == "completed"'

for run_id in "$btc_run_id" "$evm_run_id" "$report_run_id"; do
  replay_response="$(mfm_cli --output-format json run replay "$run_id" \
    --database-url "$DATABASE_URL")"
  printf '%s\n' "$replay_response" | jq -e '.data.retained_artifacts > 0'
done
```

The successful sequence produces three completed run ids, then verifies replay for all three from
retained evidence only. The BTC batch resolves one joint tip before its address reads; the EVM batch
resolves one joint tip and performs the pinned block/balance reads for each account. The report reads
Platform facts only, selects network-coherent anchors under
`mfm.portfolio.holding.latest-network-coherent.v1`, and projects pins from those selected facts.
Repeat the two collector runs when configured subjects need fresh anchors, then run the report again.

## Prove-before-write (collectors)

Before any Platform write, collectors fail closed when:

- height/hash missing
- balance not proven at the joint tip hash (BTC pin-in request; EVM hash selector + tip re-verify)
- tip drift / hash mismatch
- coverage not in `{configured_only, complete_at_anchor}`
- `source_status != ok`
- secrets / `wallet_id` / `symbol_id` would appear on fact subject/response

Multi-subject same-network batches **must** share one joint tip resolved once so
all written facts carry the same anchor.

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
