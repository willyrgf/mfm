# Known limitations

- PostgreSQL claims primary crash/restart durability only. It does not claim safe writable rollback,
  host-loss failover, quorum, replica, or multi-primary authority.
- Trusted Rust State implementations, assembly, and adapters are in the process trust base.
- Runtime has caller-driven progression only; it owns no background scheduler or timeout policy.
- The current product entry point is `mfm.portfolio/snapshot@1`.
- EVM transaction settlement version 1 is limited to the pinned non-reorging development fixture.
  Production submission remains unsupported until a product defines its finality, confirmation,
  reorg, authorization, and operational policy under a separately reviewed capability identity.
- The in-process keystore has no encrypted persistent custody or key recovery across host-process
  termination. The managed cold-recovery proof rebuilds Runtime and IO handles while retaining the
  same ephemeral signer owner.
- The append-only EVM transaction authority has no writable rollback, snapshot restoration, nonce
  release/reuse, replacement, or fee-bump contract. Loss of acknowledged authority requires a new
  authority epoch and fresh runs rather than reconstruction of its old writable timeline.
- The standalone CLI exposes no token or contract deployment, configuration effects, transaction
  submission, or keystore administration.
- Sequential fresh RunIds can grow retained history without a product quota.
- Effect progression is caller-driven and permits duplicate adapter entry for the same exact
  `EffectId` and command. Each mutating adapter must supply its own convergent durable authority.
