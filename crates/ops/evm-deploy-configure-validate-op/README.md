# mfm-op-evm-deploy-configure-validate

Composed operation (`op_id = "evm_deploy_configure_validate"`, `op_version = "v1"`) that expands into the existing:

1. `evm_deploy`
2. `evm_configure`
3. `evm_validate`

state graphs, chained sequentially inside a single op boundary.

This keeps deploy/configure/validate business orchestration in `crates/ops/*` while binaries and `crates/app` remain run-control/transport layers.
