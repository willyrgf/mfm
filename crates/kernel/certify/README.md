# mfm-certify

Sole structured program expansion and certification authority.

The pure pipeline is child substitution, capability expansion, policy wrapping, failure
completion, then normalization/certification. It validates lexical dominance, exact child
bindings, exhaustive Match, nominal outcomes, failure plans, protected boundaries, FanOut bounds,
component dependencies, policy coverage, and secret-free implementation closure.

This crate has no store dependency and no live process-invocation authority surface for production
mutation. `ProgramRegistryBuilder` registers executable/qualification objects, values, states,
capabilities, adapters, child programs, expansions, frozen entry envelopes, and process bindings.
`AdmissionCertificationRegistry` certifies a per-admission authored candidate only when its exact
semantic/implementation pairs and bounds fit that envelope. `AdmissionVerificationRegistry`
recomputes the exact qualified document for the store adapter. Finalization consumes the caller's
complete expected entry-point identity set and rejects duplicate, missing, or extra identities.

Store assembly consumes one complete `QualifiedProgramRegistry` exactly once, installing the
admission verification half in the private store adapter and the process half in Runtime.
Application code must not split and reassemble those halves independently.

Registered authored programs may be replaced only through the support-envelope API before registry
finalization. That does not grant certified-root substitution: persisted verification always
recomputes from the admitted root's authored object and content identity.

There is no compatibility lowering or alternate certifier.
