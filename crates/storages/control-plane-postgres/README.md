# mfm-control-plane-postgres

PostgreSQL-backed control-plane storage for correctness-critical stream families and projections.

Current slice:
- durable `rpc_source:*` stream-family appends
- rebuildable `mfm_rpc_source_state` projection updates in the same SQL transaction
- durable `source_pool:*` stream-family appends for catalog declaration, membership, and ranking snapshots
- rebuildable `mfm_source_pool_state` projection updates in the same SQL transaction

All persisted control-plane identities are scoped by `control_scope` plus network-specific ids:
- `rpc_source:<control_scope>:<network_id>:<source_id>`
- `source_pool:<control_scope>:<network_id>:<pool_kind>`

Persisted `source_pool:*` payloads store the non-secret catalog declaration plus stable source ids
and ranking snapshots. Endpoint URLs and auth material stay in the runtime registry and are never
written here.
