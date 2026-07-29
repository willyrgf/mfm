# mfm-certify

The sole deterministic composite planner and offline certifier for recoverability-v2 graphs.

The private composite planner expands child composition first, framework policy outside the
protected state, and executor support inside it. Final canonical paths are complete before node ids
are derived. Every dependency, effective output, public output, and terminal rule is retained in
the expanded spec.

`CompositeCertificationFactory` is zero-state. Registry assembly invokes it exactly once with the
sole shared immutable program definition; the resulting private certifier derives manifests from
that same definition and returns complete `CertifiedAdmissionArtifacts`. Candidate certification
uses the same callback followed by registry-owned output and proof-closure validation. Runtime
does not repair or expand certified graphs.

`docs/design.md` is the normative typed-core authority contract. This crate is framework-owned and
must remain domain-free.
