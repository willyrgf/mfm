# mfm-replay

Typed kernel crate for certified replay evidence brokers and verifier contracts.

`docs/design.md` is the normative typed-core authority contract.
This crate answers replay requests only from certified typed run streams, store projections, and
recorded artifact evidence. It must not construct live capabilities or live transports.

Replay authority is minted from certified spec authority, a validated `VerifiedRunHistoryView`, and
retained artifact evidence from the committed run stream plus rebuilt and validated projections.
Raw status DTOs, stream JSON, hash-only specs, rendered public output, or artifact-store bytes
without committed evidence cannot construct replay authority.

The generic pure-state verifier reconstructs certified config, arbitrary input trees, and typed
context from retained evidence, invokes the same `PureState` behavior, and compares exact canonical
output bytes. Replay does not resolve current configuration, read the current executable, mint live
execution identity, or call a provider.
