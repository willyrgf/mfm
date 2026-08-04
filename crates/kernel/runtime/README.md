# mfm-runtime

One-action interpreter and sole production path that can request semantic run-history mutation.

Runtime holds a consumer-side `RuntimeHistoryPort` and the process registry. Production adapters,
backends, and the sole fold remain private to `mfm-store` assembly. Runtime never receives a raw
backend, writer, pool, or proposal forge surface.

Runtime loads one verified cursor through the port, selects the minimum declaration-ordered
actionable occurrence, and performs at most one transition or audited external operation. It does
not maintain an in-memory scheduler or domain-specific lifecycle.

For Read/Effect states, Runtime privately owns:

```text
Prepared -> committed authorization -> affine Authorized
  -> one registered invoker -> PendingObservation
  -> committed observation -> state settlement
```

Normal completion cannot escape before observation persistence. Ambiguous authorization is
resolved before invocation, and observation retry never invokes again. Only committed Effect
`SupersededBeforeEntry` permits a next ordinal; possible entry parks and integrity evidence blocks.
The purpose-limited prior-run fact scanner follows this same ordinary Read bracket and alone
consumes the store-minted newly committed authorization permit.

Runtime commits the exact registered callback proposal. It owns no EVM/nonce knowledge, raw
backend, replay service, or public DTO rendering. Callers may implement `RuntimeHistoryPort` for
isolated tests; such ports cannot attach to MFM production backends.

Before an invoker is entered, Runtime verifies the complete committed authorization lineage:
record reference, predecessor/successor heads, attempt identity and ordinal, occurrence path,
semantic call/head, state input, access kind, capability/adapter identities, typed request
reference and digest, physical certificate, stable resource lineage, store scope, and writer
epoch. A matching request digest alone is never sufficient authority.
