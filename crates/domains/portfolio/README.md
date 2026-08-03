# mfm-portfolio

Pure portfolio model and structured snapshot operation.

`PortfolioSnapshotSelector { target }` names one configured portfolio. App resolves and validates
the current append-only `PortfolioConfig`, combines it with the admitted routing manifest, and asks
this crate to compile ordered EVM collection roots.

The operation uses a collect-all portfolio FanOut whose lanes contain the EVM-owned bounded balance
Read fan-out, producing the certified depth-two case. Aggregation receives exact ordered nominal
results and returns `PortfolioPublicOutputs` or typed `PortfolioSnapshotFailure`.

The crate performs no routing IO, store access, provider calls, signing, or replay. Bitcoin model
support remains data-only in the current production executable.
