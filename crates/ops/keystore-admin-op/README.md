# mfm-op-keystore-admin

Keystore admin operations (`op_id = "keystore_import"`, `keystore_list`, and `keystore_delete`, all `op_version = "v1"`).

Purpose:
- keep keystore admin execution in `crates/ops/*`
- keep CLI/REST as thin wrappers over run start/resume/report rendering

Outputs:
- `keystore_import` report: `id`, `label`, `key_type`, `address`, `created_at`
- `keystore_list` report: `keys[]`, `show_addresses`
- `keystore_delete` report: `id`, `label`
