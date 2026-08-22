# Known limitations

- PostgreSQL claims primary crash/restart durability only. It does not claim safe writable rollback,
  host-loss failover, quorum, replica, or multi-primary authority.
- Trusted Rust State implementations, assembly, and adapters are in the process trust base.
- Runtime has caller-driven progression only; it owns no background scheduler or timeout policy.
- The current product entry point is `mfm.portfolio/snapshot@1`.
- The generic durable Effect protocol is implemented, but production EVM transaction submission
  remains unsupported until the EVM transaction authority and live adapter are registered by a
  product surface.
- The standalone CLI exposes no token or contract deployment, configuration effects, transaction
  submission, or keystore administration.
- Sequential fresh RunIds can grow retained history without a product quota.
- Effect progression is caller-driven and permits duplicate adapter entry for the same exact
  `EffectId` and command. Each mutating adapter must supply its own convergent durable authority.
