# mfm-op-keystore-tx

Keystore transaction operations (`op_id = "keystore_tx_sign"`, `op_version = "v1"`).

Purpose:
- keep transaction signing graph composition in `crates/ops/*`
- keep local signing in `mfm-state-keystore`
- keep CLI/REST as thin wrappers over run start/resume/report rendering

Outputs:
- `keystore_tx_sign` report: `from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`, `out_path`
