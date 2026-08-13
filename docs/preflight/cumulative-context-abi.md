# Cumulative-context ABI proof

This is the checked-in U8/U11 artifact. Every row names one complete typed value crossing a State
boundary. Runtime and Store preserve the nominal contract/content identity; they do not assemble,
split, reorder, or interpret domain fields.

## EVM submission

| Boundary | Input | Output or failure | Execution |
| --- | --- | --- | --- |
| Admission | `EvmSubmissionRequest` (`C0`) | `EvmSubmissionContext` or `EvmSubmissionFailure` | Domain planning creates one `C0`; no kernel root vector |
| Read wallet nonce | `EvmSubmissionRequest` | `EvmSubmissionContext` | `ReadWalletNonceStatus`, exact request/target identity |
| Derive candidate | `EvmSubmissionContext` | `EvmSubmissionContext` | Pure, deterministic, no provider access |
| Broadcast | `EvmSubmissionContext` | `EvmSubmissionOutput` or `EvmSubmissionFailure` | `BroadcastTransaction`, `EntryOnce`; preparation fixes nonce/candidate identity before entry |
| Failure route | `EvmSubmissionFailure` | Declared terminal output route | Pure fail-fast State; no later normal State is evaluated |

The broadcast adapter binds the candidate id, sender, nonce, nonce domain, call id, operation id,
and immutable physical target. The typed EVM continuation is not recovered from history by a State.

## Portfolio snapshot

| Boundary | Input | Output or failure | Execution |
| --- | --- | --- | --- |
| Admission | `PortfolioSnapshotInput` (`C0`) | `PortfolioContinuation` | One singular domain-planned value; at most 64 total EVM sources |
| Collection entry | `PortfolioContinuation` | `EvmBalanceContext<PortfolioContinuation>` | Portfolio-owned Pure State constructs the complete child context |
| Asset selection | `EvmBalanceContext<PortfolioContinuation>` | `EvmNativeBalanceInput` or `EvmTokenBalanceInput` | Closed `Match`; selected payload is the complete first child input |
| Native/token stages | Native/token input or balance context | `EvmBalanceContext<PortfolioContinuation>` | Explicit sequential Read States; failure routes immediately |
| Collection consolidation | `EvmBalanceContext<PortfolioContinuation>` | `EvmBalanceCollectionCompletion<PortfolioContinuation>` | Pure; validates declaration order, anchor, sources, and result arithmetic |
| Collection resume | `EvmBalanceCollectionCompletion<PortfolioContinuation>` | `PortfolioContinuation` | Portfolio-owned Pure State appends exactly one completed collection |
| Final consolidation | `PortfolioContinuation` | `PortfolioSnapshotOutput` or `PortfolioSnapshotFailure` | Pure terminal State; no dynamic collection scheduler |

`PortfolioContinuation` is the opaque caller continuation carried by EVM. EVM can transport it as
an exact typed value but has no accessor for Portfolio semantics. `EvmBalanceResultMetadata`
allocates the reviewed public ordinal/correlation fields once; it is not a serialized continuation
or history handle.

## Compile and ownership proof

The non-`Clone` `Context` fixture in
`crates/kernel/runtime/src/single_trust.rs` proves catalog qualification, affine session retention,
and consuming handoff. `catalog_erases_and_downcasts_owned_non_clone_values_once` proves the
catalog-brand/type witness path. `reducer_materializes_one_selected_match_payload` proves that a
selected Match payload becomes the exact child input while a distinct terminal output remains the
only conclusion object. The Runtime panic tests prove that preparation, adapter construction, and
future polling do not detach an owner.

The remaining cross-crate reification work is deliberately isolated to the Runtime coordinator:
it must use one catalog-issued type witness and a consuming `Any` owner internally, never raw
`TypeId`, unsafe code, a public free downcast, erased bytes, or a second hot Serde round trip.
