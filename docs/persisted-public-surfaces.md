# Persisted and public surfaces

Persisted run data is a strict canonical frame stream. Each frame contains one `RunAdmitted`,
`StatePrepared`, or `StateConcluded` record plus its append-atomic object closure. Values are
content-addressed by nominal contract and exact canonical bytes. Hashed structures contain no
floating-point values.

These Journal types are public checked wire DTOs for strict decoding, replay/export, tests, and
mechanical storage conformance. Constructing one proves bounded canonical structure only; it does
not create semantic mutation authority. Supported execution accepts catalog-qualified typed values
and coordinate-free intent/outcome/evidence/fact material. Store alone supplies frame coordinates
and append identities under an affine `SelectedRun` owner.

`RunAdmitted.configuration` is the exact resolved configuration stream coordinate: its one-based
global sequence and typed content reference. It is not a caller-supplied generic content reference.
Configuration revisions are separate from run frames and retain canonical bytes, the exact
`MfmConfig` schema/content identity, and cumulative stream bytes. The retired generic
`mfm.configuration` schema has no reader.

Public App, CLI, REST, export, and error surfaces contain no credentials, private keys, raw provider
bytes, provider diagnostics, or secret-bearing context. A fixed-tenant facade derives partition
identity from trusted construction rather than caller input.

`QualifiedRun` is cloneable callback-free evidence. `HistoryReader` can return it but cannot promote
it. `SelectedRun` is non-Clone, non-Serde, has no public constructor, and can release its history
only by consuming itself. Runtime exposes neither its exact mutation port nor the opened Store.

Portable export is one canonical v5 structural envelope of complete frames. Import rechecks the
stream identity, framing, canonical bytes, sequence, identity, and the closed three-family record
algebra before a consumer receives a qualified prefix.
