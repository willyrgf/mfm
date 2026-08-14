# Cumulative-context ABI proof

This is the checked-in U8/U11 artifact. Every row names one complete typed value crossing a State
boundary. Runtime and Store preserve the nominal contract/content identity; they do not assemble,
split, reorder, or interpret domain fields.

## EVM submission

| Boundary | Input | Output or failure | Execution |
| --- | --- | --- | --- |
| Admission | `EvmSubmissionRequest` (`C0`) | `EvmSubmissionProgress` (reserved phase) or `EvmSubmissionFailure` | Domain planning creates one `C0`; no kernel root vector |
| Reserve wallet nonce | `EvmSubmissionRequest` | `EvmSubmissionProgress` (reserved phase) | One exact one-entry nonce-reservation Effect for the target's tenant/sender/nonce domain |
| Derive candidate | `EvmSubmissionProgress` (reserved phase) | `EvmSubmissionProgress` (candidate phase) | Pure, deterministic, no provider access |
| Broadcast | `EvmSubmissionProgress` (candidate phase) | `EvmSubmissionProgress` (broadcast phase) or `EvmSubmissionFailure` | One exact one-entry broadcast Effect; preparation fixes every transaction/signer field before entry |
| Receipt/finality suffix | broadcast -> receipt -> finalized phases of `EvmSubmissionProgress` | `EvmSubmissionProgress` (canonical phase) | Exact receipt, finalized-head, and canonical-inclusion Reads authenticate disposition evidence |
| Consolidate | `EvmSubmissionProgress` (canonical phase) | `EvmSubmissionOutput` or `EvmSubmissionFailure` | Pure; only a matching finalized canonical receipt projects `succeeded` or `reverted` |
| Failure route | declared EVM failure | Durable terminal failure | No fake success State or later normal work is evaluated |

`EvmSubmissionProgress` is the one public State-boundary value; its closed internal phase retains
the exact reservation, candidate, broadcast, receipt, finality, or canonical proof. The broadcast
adapter binds the candidate id, sender, nonce, nonce domain, call id, operation id, and immutable
physical target. The typed EVM continuation is not recovered from history by a State.

## Portfolio snapshot

| Boundary | Input | Output or failure | Execution |
| --- | --- | --- | --- |
| Admission | `PortfolioSnapshotInput` (`C0`) | `PortfolioContinuation` | One singular domain-planned value; at most 64 total EVM sources |
| Collection entry | `PortfolioContinuation` | `EvmBalanceContext<PortfolioContinuation>` | Portfolio-owned Pure State constructs the complete child context |
| Asset selection | `EvmBalanceContext<PortfolioContinuation>` | `EvmBalanceAsset<PortfolioContinuation>` | Closed `Match`; either arm retains the byte-identical complete child context |
| Native/token stages | Complete balance context | `EvmBalanceContext<PortfolioContinuation>` | Explicit sequential Read States retain checked chain, common initial anchor, asset, decimal scale, and raw units; failure routes immediately |
| Collection consolidation | `EvmBalanceContext<PortfolioContinuation>` | `EvmBalanceCollectionCompletion<PortfolioContinuation>` | Pure; validates declaration order, anchor, sources, and result arithmetic |
| Collection resume | `EvmBalanceCollectionCompletion<PortfolioContinuation>` | `PortfolioContinuation` | Portfolio-owned Pure State appends exactly one completed collection |
| Final consolidation | `PortfolioContinuation` | `PortfolioSnapshotOutput` or `PortfolioSnapshotFailure` | Pure terminal State; no dynamic collection scheduler |

`PortfolioContinuation` is the opaque caller continuation carried by EVM. EVM can transport it as
an exact typed value but has no accessor for Portfolio semantics. `EvmBalanceResultMetadata`
allocates the reviewed public ordinal/correlation fields once; it is not a serialized continuation
or history handle. Completed balance results, active work, and the remaining declaration suffix are
derived and validated as one exact ordered demand rather than stored as redundant cursors.
EVM recomputes every completed integer-scaled amount from its raw units and source scale; Portfolio
recomputes the completed collection total against its admitted collection scale before projecting it.

## Compile and ownership proof

The non-`Clone` assembly-cutover fixture proves catalog qualification, affine session retention, and
consuming handoff. Program's association regressions prove that the exact nominal contract,
descriptor identity, Rust type, and catalog instance must all agree, including when two Rust types
claim one schema. The retained-byte regression proves one strict decode. Runtime assembly tests
reject missing, surplus, and wrong typed registrations before callbacks can run. The Store Match
regression proves that a selected Match payload reuses the exact retained value object. The Runtime
panic tests prove that preparation, adapter construction, and future polling do not detach an owner.

The Runtime coordinator now uses one catalog-issued type witness and a consuming branded erased
owner internally; raw `TypeId` is only private live-implementation correlation, and there is no
unsafe code, public free downcast, or erased context map. `RunSession` retains the latest typed value
together with one affine Store-selected `SelectedRun` across hot advancement.
Cloneable `QualifiedRun` evidence has no reader-to-selection promotion path; releasing it consumes
the selected owner. The lifecycle regression advances a two-State Pure chain
from head 1 to head 2 without re-reducing the prefix, drops the hot owner, and cold-resumes the
same durable context to head 3. The access lifecycle regression counts provider entry, response
ingress, preparation, and interpretation separately and observes exactly one of each. No remaining
cross-crate context reification work is tracked by this artifact.
