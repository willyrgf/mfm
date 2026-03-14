# mfm-transports-rpc-control

Live `rpc.control` transport for managed EVM RPC routing.

This crate is the runtime bridge between:

- the typed caller-facing `mfm-collectors-rpc-control` client
- the durable Postgres control-plane store
- the existing `mfm-collectors-evm-jsonrpc-http` executor

It owns bootstrap source catalog parsing, idempotent-ish source-pool preparation, durable
health/ranking updates, and source selection for unpinned managed calls.
