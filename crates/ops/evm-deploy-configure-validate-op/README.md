# mfm-op-evm-deploy-configure-validate

This crate owns the certified typed deploy/configure/validate workflow plan.

The operation lowers canonical EVM DCV config into three typed lifecycle states:

- `DeployContractState`: managed deployment side effect producing `DeployedContract`
- `ConfigureContractState`: managed configuration side effect consuming `DeployedContract` and producing `ConfiguredContract`
- `ValidateContractState`: read-only validation consuming `ConfiguredContract` and producing `ValidationReport`

The typed operation emits a certified execution spec with typed config artifacts and public output
metadata. It does not expose legacy dynamic root ops, context-key artifact ports, or child graph
planning as semantic runtime APIs.
