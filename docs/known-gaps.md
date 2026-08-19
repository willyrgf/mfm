# Known limitations

- PostgreSQL claims primary crash/restart durability only. It does not claim safe writable rollback,
  host-loss failover, quorum, replica, or multi-primary authority.
- Trusted Rust State implementations, assembly, and adapters are in the process trust base.
- Runtime has caller-driven progression only; it owns no background scheduler or timeout policy.
- The current product entry point is `mfm.portfolio/snapshot@1`.
- EVM transaction submission remains unsupported until a future RFC defines durable transaction
  authority and outbox semantics.
- The standalone CLI admits one EVM route per configuration file, so a configuration whose
  collections span two chain IDs cannot be planned. It exposes no `resume` command, no token or
  contract deployment, no configuration effects, and no keystore import.
- The REST binary only prints an unavailable diagnostic and exits; it binds no listener and exposes
  no route.

## Deferred Effect symmetry

The typed authoring DSL deliberately exposes only executable Pure and Read occurrences today. A
future durable Effect cut must extend the same `OperationExpansion` compiler rather than add a
second builder or lower mutation through Read semantics. Its expected source-level symmetry is:

```rust,ignore
pub fn effect<S, C>(
    &mut self,
    setup: &<C as CapabilityInjection<S>>::Setup,
) -> Result<()>
where
    S: EffectState<C>,
    C: EffectCapabilityContract + CapabilityInjection<S>;
```

If a capability policy injects a mutating support occurrence such as nonce reservation, the same
cut must also add a restricted `InjectionWriter::effect`. Expansion remains deterministic and
IO-free: it may author a `ReserveNonce` Effect State, but it must never reserve a nonce, sign, or
call a provider while constructing the Program.

These names and bounds are a design reminder, not a current or reserved API contract. The future
RFC must define and land the complete boundary atomically:

- `EffectState<C>` and `EffectCapabilityContract`, including the command/result or
  command/evidence ABI;
- the Effect Program execution discriminant and any schema/version cutover;
- Runtime registration, association, driver, and adapter ownership;
- durable pre-provider authorization or outbox state, command identity, idempotency, replay
  resistance, nonce fencing, and exact signed-byte custody;
- definite, ambiguous, and reconciled provider acknowledgement plus crash/cancellation recovery;
  and
- Effect injection continuity, failure routing, depth, and restart tests.

Do not add an always-rejecting placeholder, represent Effect as Read, or treat authored injection
as proof that a nonce was reserved or a mutation was authorized. In particular, the design must
handle both provider acceptance followed by a local timeout and a crash after nonce reservation
but before durable linkage to the exact command.
