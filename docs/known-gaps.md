# Known limitations

- The admitted PostgreSQL durability profile covers primary crash/restart only. It makes no host
  loss, failover, replica, quorum, or multi-primary claim.
- Trusted Rust State implementations and adapters are inside the process trust base; untrusted
  plugins require a separate process or enforceable sandbox.
- Dropping a process-local owner before a conclusion is durable deliberately leaves only the
  durable prefix. The design does not claim rollback resistance after every later head anchor is
  lost.
- The current product fixes the two entry points `mfm.portfolio/snapshot@1` and
  `mfm.evm/submit-transaction@1`. Additional workflows require a new design decision.
- The fixed-tenant `Application` facade currently exposes callback-free structural drive/read/
  replay operations. Trusted embeddings register live State and adapter implementations through
  `mfm-runtime`; the facade does not manufacture a live registry or publish a successful result
  before its matching durable conclusion.
