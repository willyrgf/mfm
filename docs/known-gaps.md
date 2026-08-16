# Known limitations

- PostgreSQL claims primary crash/restart durability only. It does not claim safe writable rollback,
  host-loss failover, quorum, replica, or multi-primary authority.
- Trusted Rust State implementations, assembly, and adapters are in the process trust base.
- Runtime has caller-driven progression only; it owns no background scheduler or timeout policy.
- The current product entry point is `mfm.portfolio/snapshot@1`.
- EVM transaction submission remains unsupported until a future RFC defines durable transaction
  authority and outbox semantics.
- The standalone CLI exposes one-shot help/version metadata. The REST binary only prints an
  unavailable diagnostic and exits; it binds no listener and exposes no route.
