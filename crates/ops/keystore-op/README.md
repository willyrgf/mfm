# mfm-op-keystore

Keystore operations:

- admin operations: `keystore_import`, `keystore_list`, and `keystore_delete`
- transaction operation: `keystore_tx_sign`
- all operations use `op_version = "v1"`

Purpose:
- keep keystore planning in `crates/ops/keystore-op`
- keep keystore execution and secret-adjacent behavior in `mfm-state-keystore`
- keep local prompt/filesystem behavior in `mfm-transports-local-keystore`
- keep CLI/REST as thin wrappers over run start/resume/report rendering

Outputs:
- `keystore_import` report: `id`, `label`, `key_type`, `address`, `created_at`
- `keystore_list` report: `keys[]`, `show_addresses`
- `keystore_delete` report: `id`, `label`
- `keystore_tx_sign` report: `from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`
