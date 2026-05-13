# mfm-op-keystore-tx

Keystore transaction operations (`op_id = "keystore_tx_sign"`, `op_version = "v1"`).

Purpose:
- keep transaction signing graph composition in `crates/ops/*`
- keep local signing in `mfm-state-keystore`
- keep CLI/REST as thin wrappers over run start/resume/report rendering

Outputs:
- `keystore_tx_sign` report: `from`, `to`, `nonce`, `chain_id`, `tx_type`, `payload_hash`

Output files:
- default write policy is `create_new`; existing output paths fail closed
- callers must opt into `overwrite` to replace an existing regular file
- the local keystore transport rejects symlink outputs and unsafe parent directories, writes through a same-directory temporary file, and installs the final file with restrictive permissions
