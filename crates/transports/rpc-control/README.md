# mfm-transports-rpc-control

Live `rpc.control` transport for managed EVM RPC routing.

This crate is the runtime bridge between:

- the typed caller-facing `mfm-collectors-rpc-control` client
- the backend-neutral `mfm-control-plane-model` records and projections
- the durable Postgres control-plane store
- the existing `mfm-collectors-evm-jsonrpc-http` executor

It owns bootstrap source catalog parsing, idempotent-ish source-pool preparation, durable
health/ranking updates, and source selection for unpinned managed calls.

Canonical managed requests must carry explicit `network_id`. Effective request identity
includes `control_scope`, with `shared` allowed only as a caller-side default that is
stamped into the serialized request before fact-key derivation. Durable control-plane
state is keyed by `control_scope` plus network/source dimensions, and every managed
bootstrap source must declare `network_id`.
