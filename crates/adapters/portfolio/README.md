# mfm-adapters-portfolio

Portfolio adapter runners for **fact-backed, receipt-pinned** portfolio snapshots.

This crate binds certified portfolio state descriptors to executable typed runners. It receives
store-verified input evidence, artifact-read capabilities, and a Platform `FactIndexReadProvider`
from runtime/app assembly.

## Live path

1. `SelectHoldings` consumes one exact `PortfolioCollectionReceipt`, queries only its certified
   source/descriptor/anchor/coverage/status predicates with the closed N + 1 bound, hydrates every
   retained `FactResponse` artifact via the shared kit, rederives each identity, and orders only
   identity-matching candidates. It records the bounded query evidence for replay.
2. Downstream pure states assemble valuations, snapshot, and report from certified config and the
   selected holdings.

There is **no** live portfolio transport factory, pin/observe runners, or chain crawl path.

## Replay path

`verify_portfolio_replay` recomputes the **full pure graph** from certified node configs and
verified fact-query evidence, then compares each produced cell byte-for-byte:

| State | Replay behavior |
| --- | --- |
| operation-local collection receipt | recompute exact manifest/family receipt fan-in and compare its output |
| `ResolveSubjects` | recompute from certified config (not history deserialization alone) |
| `SelectHoldings` | recompute bounded cardinality, hydration, identity filtering, and post-identity ordering from recorded query evidence |
| `ResolveValuations` | recompute from certified valuation config |
| `AssembleSnapshot` | recompute from recomputed subjects/holdings/valuations and prove it consumed the exact collection receipt |
| `ProjectReport` | recompute from assembled snapshot + report config |

Replay and resume authority remains with the certified spec and typed run stream. This crate does
not create workflow topology or own collector source IO.
