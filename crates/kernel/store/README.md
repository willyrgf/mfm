# mfm-store

Sole callback-free fold, private production history adapter, and purpose readers for structured
runs.

The fold validates complete raw history, re-certifies the persisted program via the concrete
`AdmissionVerificationRegistry`, enforces writer lineage and exact heads, checks record logical
keys/cursor legality/access linkage/lexical provenance, validates exact object and fact closure,
and derives one `VerifiedStructuredRun`. That view contains chronology, semantic/journal heads,
lexical bindings, fan-out cursors, waiting or blocked state, and terminal outcome.

Production assembly consumes one complete `QualifiedProgramRegistry` and a backend, then returns
only `Runtime` plus sealed purpose readers (public-read, trace, audit, replay, export). Each
purpose reader returns only its purpose-sealed evidence newtype (`PublicRunEvidence`,
`TraceRunEvidence`, `AuditRunEvidence`, `RecordedRunEvidence`, `ExportRunEvidence`); none expose
complete `VerifiedStructuredRun` publicly, and cross-purpose evidence substitution is a type error.
Assembly never returns a store, writer, port, backend, pool, proposal constructor, or append
attempt. Semantic mutation is reachable only through Runtime's `RuntimeHistoryPort`, implemented by
a private adapter.

`RunId` is derived inside the adapter from the annex `mfm.run-id-preimage.v1` (store scope, tenant
scope, entry-point operation ID, invocation identity). Callers never supply a trusted run digest.

On certified-root cache miss the adapter calls `AdmissionVerificationRegistry::verify_root` and
consumes private-field `CertifiedProgram` data. There is no public verifier trait or verified-program
constructor.

Non-empty fact transitions atomically advance a dense store/epoch/tenant publication frontier.
The exact reserved prior-run fact Read captures that frontier, and only its newly committed
authorization receives a one-use purpose-limited scan permit.

The crate also defines a separate append-only configured-value history keyed by store, tenant,
entry operation, and target. Deployment owns its writer; application uses a reader. Configuration
is not a sixth run record family.

The memory backend is conformance/test support. Production PostgreSQL lives in
`mfm-storage-postgres` and must implement identical semantics with real SQL.
