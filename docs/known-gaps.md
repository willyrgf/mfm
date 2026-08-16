# Known limitations

- The admitted PostgreSQL durability profile covers primary crash/restart only. It makes no host
  loss, failover, replica, quorum, or multi-primary claim.
- Trusted Rust State implementations and adapters are inside the process trust base; untrusted
  plugins require a separate process or enforceable sandbox.
- Dropping a process-local owner before a conclusion is durable deliberately leaves only the
  durable prefix. The design does not claim rollback resistance after every later head anchor is
  lost.
- The current product fixes one entry point, `mfm.portfolio/snapshot@1`. Transaction submission
  remains unsupported until a future RFC defines durable transaction authority and an outbox.
