# mfm-runtime

One-action interpreter and sole production holder of the structured RunHistory writer.

Runtime loads one verified cursor, selects the minimum declaration-ordered actionable occurrence,
and performs at most one transition or audited external operation. It does not maintain an
in-memory scheduler or domain-specific lifecycle.

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
backend, replay service, or public DTO rendering.
