# mfm-op-keystore-tx

Keystore transaction operations (`op_id = "keystore_tx_sign"` and `op_id = "keystore_tx_send_raw"`, both `op_version = "v1"`).

Purpose:
- keep tx signing/send domain execution in `crates/ops/*`
- keep CLI/REST as thin wrappers over run start/resume/report rendering

Outputs:
- `keystore_tx_sign` report: `from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`, `out_path`
- `keystore_tx_send_raw` report: `tx_hash`, `rpc_url_host`, `submitted_at`
