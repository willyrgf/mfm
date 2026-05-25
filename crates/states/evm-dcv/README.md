# mfm-state-evm-dcv

Typed EVM deploy/configure/validate lifecycle state contracts.

This crate owns the typed deploy, configure, and validate state specs plus the non-secret lifecycle
values they produce and consume. Live transaction submission, validation reads, artifact staging,
and replay verification are supplied by `mfm-transports-evm-dcv`.

State contracts:

- `DeployContractState`
- `ConfigureContractState`
- `ValidateContractState`

Raw signed transaction bytes remain below the typed semantic value boundary. Runners may stage
protected transaction material as managed artifacts, but public typed values and public outputs must
contain only non-secret lifecycle evidence.
