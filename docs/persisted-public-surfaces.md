# Persisted and public surfaces

Persisted run data is a strict canonical frame stream. Each frame contains one `RunAdmitted`,
`StatePrepared`, or `StateConcluded` record plus its append-atomic object closure. Values are
content-addressed by nominal contract and exact canonical bytes. Hashed structures contain no
floating-point values.

Public App, CLI, REST, export, and error surfaces contain no credentials, private keys, raw provider
bytes, provider diagnostics, or secret-bearing context. A fixed-tenant facade derives partition
identity from trusted construction rather than caller input.

Portable export is one canonical v5 structural envelope of complete frames. Import rechecks the
stream identity, framing, canonical bytes, sequence, identity, and the closed three-family record
algebra before a consumer receives a qualified prefix.
