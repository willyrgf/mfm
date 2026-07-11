# Portfolio collect-then-report

Collectors write Platform data facts. `portfolio_snapshot` is **report-only** (SelectHoldings over
admitted facts). There is **no** dual live path and **no** mixed certified draft that expands
collectors inside the report op.

This document is the permanent authority contract and operator recipe for fact-backed portfolio
collection and reporting.

## Authority split

| Surface | Role |
|---|---|
| `btc_address_balance` / `evm_native_balance` | Observe chain (joint tip + pin-in balances) → record Platform facts |
| `btc_chain_head_collector_cycle` | Control checkpoint / chain-head observation only — **not** report pin authority |
| `portfolio_snapshot` | Resolve subjects → SelectHoldings → valuations → assemble → project report |

Forbidden:

- Mixed certified draft: collectors + report in one `portfolio_snapshot` expand
- Report constructing live BTC/EVM balance providers
- Collectors Platform-querying prior portfolio holdings for observe
- Optional shared tip for multi-subject same-network batches (shared tip is **required**)
- Report pin tables, pin-facts as report authority, or live `PinViews`
- Public-facts CLI/REST as report selection authority
- Soft-success partial reports (empty wallets / zeroed required holdings on success)

## As-of and network pins (single authority model)

| Concept | Role | Report as-of authority? |
|---|---|---|
| Fact anchor (height + block hash on holding responses) | Source-near claim content | **Yes** |
| Certified selection policy | Hash-defining rule choosing acceptable facts | **Yes** |
| Store read frontier on fact-query receipt | Records which store watermark the query read | Query evidence only |
| Public `network_pins` | Projection of selected required holding anchors | **No** — descriptive output only |
| Progressive chain-head / Control cursor facts | Collector ops | **Not** report pin authority |

Rules:

1. Every Platform holding fact carries height **and** block hash. Height-only is not allowed for
   cutover BTC/EVM holding kinds.
2. The report selects facts under
   `mfm.portfolio.holding.latest-network-coherent.v1` (newest common acceptable anchor per network).
3. Public `network_pins` are **always** projected from selected required holding anchors — never
   from report config, live head, or pin-facts.
4. Same-network selected holdings share one anchor by construction of the selection policy, or the
   run fails if no common acceptable anchor exists.
5. Optional later historical bounds are filters on the **same** selection policy family, not a
   second pin store.

```text
collectors: observe(source @ joint anchor) → Platform data facts
report:     certified policy → SelectHoldings → Observations + network_pins(from selected anchors)
replay:     recorded fact-query evidence only
```

## Report fact selection path

```text
compile canonical fact query plan (certified report config + subject projection + policy)
  → FactIndexRead (Platform audience) over the requirement set
  → deterministic unsigned receipt + selection evidence
  → hydrate retained FactResponse artifact(s)
  → SelectHoldings (network-coherent policy over full acceptable candidate sets)
  → normalize into Observation (including fact anchor)
```

Rules:

- Prefer one shared read frontier/receipt for the requirement set when possible.
- Candidate selection uses **full acceptable sets**. `limit=1` is never selection authority.
- Same subject + same anchor + multiple claims: LWW via `store_commit_order`.
- Hydration is mandatory: receipt alone is not enough when the balance payload lives in the
  retained response artifact.
- Replay uses recorded fact-query evidence and retained response artifacts only. It must not
  re-query the live fact-index frontier or construct live chain providers.
- Pure report states recompute from certified config; SelectHoldings recomputes from retained
  evidence. Full graph: ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot →
  ProjectReport (see `crates/adapters/portfolio/README.md`).

Fact subjects stay source-near (network, address, chain id, …). Report joins back to `wallet_id` /
`symbol_id` only when building Observation DTOs.

Cutover holding kinds:

| Requirement | Fact kind | Subject fields (source-near) | Writable coverage |
|---|---|---|---|
| BTC native | `bitcoin.address_balance_snapshot` | network, bitcoin_network, semantic_source_identity, address | `configured_only`, `complete_at_anchor` |
| EVM native | `evm.address_native_balance_snapshot` | network, chain_id, account | `configured_only`, `complete_at_anchor` |

## Fail-closed error codes

Product cutover codes (hard-fail the select/run for required symbols):

| Code | When |
|---|---|
| `missing_fact` | no acceptable Platform fact after filters (includes unacceptable coverage/status) |
| `no_common_network_anchor` | acceptable facts exist but empty common-anchor intersection |
| `unsupported_requirement` | portfolio requirement has no projection rule |
| `ambiguous_facts` | selection cardinality violated after policy / receipt plan |

Residual guard (not a product soft path): `inconsistent_network_anchors`.

There is no soft-success public snapshot path and no zeroed required holdings on success.

## Prove-before-write (collectors)

Before any Platform write, collectors fail closed when:

- height/hash missing
- balance not proven at the joint tip hash (BTC pin-in request; EVM hash selector + tip re-verify)
- tip drift / hash mismatch
- coverage not in `{configured_only, complete_at_anchor}`
- `source_status != ok`
- secrets / `wallet_id` / `symbol_id` would appear on fact subject/response

Multi-subject same-network batches **must** share one joint tip resolved once so all written facts
carry the same anchor.

## External multi-run recipe (dual mainnet natives)

Operators run collectors, then report, as separate certified runs. The tracked configs are
`examples/configs/btc-address-balance.toml`, `examples/configs/evm-native-balance.toml`,
`examples/configs/portfolio-dual-mainnet.toml`, and
`examples/configs/runtime-dual-mainnet.toml`.

The following Bash sequence is the copy/pasteable operator path. It assumes `DATABASE_URL` points
at a migrated Postgres store, the RPC environment variables referenced by the runtime config are
set, and `jq` is installed.

```bash
export MFM_RUNTIME_CONFIG_FILE="$PWD/examples/configs/runtime-dual-mainnet.toml"

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
    --op-version 1 \
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

## Report never live-reads

`portfolio_snapshot` and portfolio adapters use fact-index reads only. They do not construct live
BTC/EVM chain providers for balances. Missing facts fail closed (`missing_fact` /
`no_common_network_anchor`); there is no soft-success partial report.

## Operational note on tip races

If multi-subject same-network collects run without a shared in-batch tip (for example separate
single-address runs at different tips), network-coherent selection may fall back to an older common
anchor or fail with `no_common_network_anchor`. Prefer one multi-address/multi-account batch per
network so the newest tip is common.

## Related docs

- `docs/design.md` — runtime, store, evidence-only assembly, pure/read/side-effect replay
- `docs/persisted-public-surfaces.md` — fact index as rebuildable cache vs stream authority
- `docs/architecture.md` — crate placement taxonomy
- `docs/btc-rpc-routing.md` / `docs/evm-rpc-routing.md` — transport routing
- `crates/adapters/portfolio/README.md` — full pure-graph replay verification
- `bin/cli/README.md` — public status/stream error-code surface
