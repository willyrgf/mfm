# mfm-control-plane-postgres

PostgreSQL-backed control-plane storage for correctness-critical stream families and projections.

Current slice:
- durable `rpc_source:*` stream-family appends
- rebuildable `mfm_rpc_source_state` projection updates in the same SQL transaction
