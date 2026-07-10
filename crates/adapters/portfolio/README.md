# mfm-adapters-portfolio

Portfolio adapter runners for **fact-backed, report-only** portfolio snapshots.

This crate binds certified portfolio state descriptors to executable typed runners. It receives
store-verified input evidence, artifact-read capabilities, and a Platform `FactIndexReadProvider`
from runtime/app assembly.

## Live path

1. `SelectHoldings` hydrates retained `FactResponse` artifacts via the shared kit
   (`fact_response_artifact_requirement`, `hydrate_fact_response_json`), runs pure network-coherent
   selection, and records fact-query evidence for replay.
2. Downstream pure states assemble valuations, snapshot, and report from certified config and the
   selected holdings.

There is **no** live portfolio transport factory, pin/observe runners, or chain crawl path.

## Replay path

`verify_portfolio_replay` recomputes the **full pure graph** from certified node configs and
verified fact-query evidence, then compares each produced cell byte-for-byte:

| State | Replay behavior |
| --- | --- |
| `ResolveSubjects` | recompute from certified config (not history deserialization alone) |
| `SelectHoldings` | recompute from recorded query evidence + selection policy |
| `ResolveValuations` | recompute from certified valuation config |
| `AssembleSnapshot` | recompute from recomputed subjects/holdings/valuations |
| `ProjectReport` | recompute from assembled snapshot + report config |

Replay and resume authority remains with the certified spec and typed run stream. This crate does
not create workflow topology or own collector source IO.
