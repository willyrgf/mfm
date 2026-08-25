# mfm-evm-transaction-authority

This crate is the append-only port between deterministic EVM Effect contracts and concrete nonce,
prepared-transaction, and settlement custody. Its records are checked non-Program values: none is
serializable, and exact signed bytes have no text or debug rendering.

One nonce domain is exactly authority epoch, chain ID, expected genesis hash, and sender. Custody
provider and endpoint identities are not nonce dimensions. Implementations expose only load,
reserve-or-compare, retain-prepared, and
retain-settlement. They cannot mutate, delete, roll back, activate, or broadcast facts.

`load` returns the complete retained fact without applying caller command semantics.
`reserve_or_compare` owns the atomic nonce choice and returns its authoritative reservation. The
live executor supplies complete nested `PreparedRecord` and `SettledRecord` candidates; exact
existing candidates succeed, while a different qualified first-winner conflict is `Unavailable`
and must be resolved by reloading. Each reservation retains Runtime's exact command value ref, not
the command schema's nominal contract ref.
