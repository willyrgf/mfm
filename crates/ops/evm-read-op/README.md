# mfm-op-evm-read

Read-only EVM op (`op_id = "evm_read"`, `op_version = "v1"`).

Exports are conditional:

- default config exports both `chain_id` and `block_number`
- `include_chain_id = false` omits the `chain_id` state and export
- `include_block_number = false` omits the `block_number` state and export
- disabling both queries is rejected during planning

Docs: [`../../../docs/design.md`](../../../docs/design.md)
