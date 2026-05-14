# mfm-control-plane-model

Backend-neutral model for control-plane stream families.

This crate owns:

- typed `rpc_source:*` and `source_pool:*` references
- durable stream-family record payloads
- non-secret source-pool catalog snapshots and fingerprints
- rebuildable RPC-source and source-pool projection states

Concrete persistence crates, including `mfm-control-plane-postgres`, depend on this crate for
model semantics and keep database-specific DDL, transactions, and projection table IO outside the
model boundary.
