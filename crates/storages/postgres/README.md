# mfm-storage-postgres

Durable PostgreSQL implementations of the append-only Store, mechanical RunIndex, and opaque config
catalog. The run-history and `mfm.config-catalog-postgres.v1` catalog schemas have independent
connection gates, so either surface remains usable when the other is incompatible.

Production constructors accept only a bounded private locator. It contains one single-host
`postgresql` URI with an explicit password and `sslmode=verify-full`, plus either exact compiled
WebPKI roots or an exclusive content-pinned PEM bundle. The parser and the pinned SQLx seam exclude
environment, home, passfile, service-file, socket, client-certificate, and additive-root inputs.
Every runtime connection must authenticate as the fixed `mfm_runtime` role and pass the exact role,
ownership, database/schema, table-privilege, durability, and schema gate.

Loads use one read-only repeatable-read snapshot, prove a nonempty gap-free prefix and byte/digest
accounting, then copy owned rows in one pure blocking job. Appends copy the candidate before BEGIN,
force `synchronous_commit=on`, take the per-RunId advisory transaction lock before state reads,
validate head/target, insert immutable bytes, update the head, and COMMIT.

Pre-COMMIT failures are definite typed capacity/corruption/unavailability. Only an IO/protocol loss
after COMMIT submission is `Indeterminate`.

Catalog insert and conditional delete serialize the fixed 256-entry quota, force synchronous
COMMIT, and distinguish definite failure from ambiguous acknowledgement. Listing uses bytewise
ascending keyset pages; it does not claim a snapshot across requests.

`provision_schemas` is the one public provisioning entry and is not held by runtime composition. It
requires distinct typed admin/runtime locators for the same normalized target and an already-created
fixed runtime role. The short-lived admin installs both baselines only when their namespaces are
absent, owns the objects, and grants only the exact DML authority. Existing installations are
verified and never migrated, repaired, re-owned, reset, or downgraded. Managed same-crate tests run
serially through `postgres-test` against a real TLS server and split authority.
