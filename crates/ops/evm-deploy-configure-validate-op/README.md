# mfm-op-evm-deploy-configure-validate

This crate owns the additive public deploy/configure/validate workflow boundaries:

- `evm_deploy_configure_validate_config_build/v1`: canonical config build root that publishes
  built config plus canonical/built config artifacts and a stable build report
- `evm_deploy_configure_validate_execute/v1`: strict built-config execution root
- `evm_deploy_configure_validate/v1`: canonical public root that composes build then execute

`evm_deploy_configure_validate_config_build` keeps authored/canonical transport concerns separate
from execution by validating the lowered execution payload, publishing the canonical and built
artifacts, and emitting a stable build report.

`evm_deploy_configure_validate_execute` consumes the pre-built execution config and lowers it into
the existing `evm_deploy`, `evm_configure`, and `evm_validate` child graphs.

`evm_deploy_configure_validate` accepts canonical config and composes the config-build step before
the strict execute root.

Deploy and configure phases require `signing_key_env`. The execution graph rejects node-managed
unsigned transaction submission because write states must record durable signed transaction intents
before broadcasting.
