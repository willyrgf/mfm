# EVM contract states

The reusable EVM contract surface consists of three composable state descriptors:

- `ContextBoundDeployContractState` produces a context-bound deployed instance.
- `ContextBoundConfigureContractState` accepts only that deploy descriptor's output and produces a configured instance.
- `ContextBoundValidateContractState` accepts only that configure descriptor's output and produces a validation report.

These are library states, not public operations, setup kinds, or application entry points. A
certified graph that uses them must declare the same EVM contract context for every node, retain
the configured typed inputs, and bind the contract-state adapter supplied by
`mfm-adapters-evm-contracts`.

Deploy and configure are side effects with the signer account nonce resource claim. Validation is
a read-only state. Adapter runners retain redacted provider evidence and support evidence-only
replay; no live endpoint, signer material, catalog row, or app entry-point registration is replay
authority.

The configured instance carries direct deploy-to-configure lineage only. It does not accept an
externally adopted value or a continuation from another run. Program certification enforces the
allowed producer descriptors in addition to Rust input typestate.
