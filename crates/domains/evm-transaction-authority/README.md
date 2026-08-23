# mfm-evm-transaction-authority

This crate is the append-only port between deterministic EVM Effect contracts and concrete nonce,
prepared-transaction, and settlement custody. Its records are checked non-Program values: none is
serializable, and exact signed bytes have no text or debug rendering.

One nonce domain is exactly authority epoch, chain ID, expected genesis hash, and sender. Custody
provider and endpoint identities are not nonce dimensions. Implementations expose only load,
reserve-or-compare, retain-prepared, and
retain-settlement. They cannot mutate, delete, roll back, activate, or broadcast facts.
