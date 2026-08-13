# mfm-values

`mfm-values` owns the bounded, secret-free `MfmValue` representation used by domain contexts,
capability intents/evidence, configuration values, and public outputs. It also owns strict schema
shape construction and canonical-value validation used by the derive macro.

Values are valid by representation: floats, unbounded collections, malformed identifiers, secret
markers, and non-canonical JSON are rejected at the typed boundary. The crate contains no run
history, Store, Runtime, adapter, tenant, or application policy authority.

`docs/design.md` is the normative typed-core contract.
