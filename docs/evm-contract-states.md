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

Every successful mutation receipt carries its canonical block number and block hash. The configured
value carries a `ConfiguredContractAnchor`: the last successful configure receipt in certified
transaction order, or the deploy receipt for a zero-transaction configuration. Confirmation reads
the canonical block at that number, compares its hash to the retained anchor, and proves the
certified finality depth before validation can use it.

If `ContractProfile.deployed_code_hash` is present, validation reads runtime code only at that
anchor with EIP-1898 `requireCanonical: true`. Its report projects byte length and Keccak-256 hash;
the content-addressed external evidence retains the raw bytecode, exact selector, and redacted
source binding for replay. Empty code and a mismatched hash produce `valid: false`; malformed,
unauthenticated, source-mismatched, or anchor-mismatched evidence fails execution. If the profile
does not specify a code hash, validation makes no code read and carries no placeholder evidence.

The configured instance carries direct deploy-to-configure lineage only. It does not accept an
externally adopted value or a continuation from another run. Program certification enforces the
allowed producer descriptors in addition to Rust input typestate.
