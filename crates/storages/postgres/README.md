# mfm-storage-postgres

Durable PostgreSQL implementations of the append-only Store, mechanical RunIndex, and opaque config
catalog. The run-history and `mfm.config-catalog-postgres.v2` catalog schemas have independent
connection gates, so either surface remains usable when the other is incompatible.

Production constructors accept only a bounded private locator. It contains one `postgresql` URI
with an explicit password, numeric `127.0.0.1` or `::1` host, and `sslmode=disable`. PostgreSQL is
intentionally plaintext inside the trusted shared network namespace and no remote database target
is supported. The parser uses stock SQLx, overwrites every ambient-derived connection value that
can affect this plaintext connection, and rejects `PGOPTIONS`, whose startup effects SQLx cannot
clear. Home/passfile and service-file inputs cannot influence the resulting authority. The complete
v1 wire is `{"v":1,"url":"..."}`. SQLx remains an unmodified crates.io dependency so its
compile-time query macros can be enabled when the query surface adopts them.
Every runtime connection must authenticate as the fixed `mfm_runtime` role and pass the exact role,
ownership, database/schema, table-privilege, durability, and schema gate.

Loads use one read-only repeatable-read snapshot, prove a nonempty gap-free prefix and byte/digest
accounting, then copy owned rows in one pure blocking job. Appends copy the candidate before BEGIN,
force `synchronous_commit=on`, take the per-RunId advisory transaction lock before state reads,
validate head/target, insert immutable bytes, update the head, and COMMIT.

Pre-COMMIT failures are definite typed capacity/corruption/unavailability. Only an IO/protocol loss
after COMMIT submission is `Indeterminate`.

Catalog insert and replacement serialize the fixed 256-entry quota, force synchronous COMMIT, and
distinguish definite failure from ambiguous acknowledgement. Listing returns the complete bounded
catalog in ascending bytewise name order.

`provision_schemas` is the one public provisioning entry and is not held by runtime composition. It
requires distinct typed admin/runtime locators for the same normalized target and an already-created
fixed runtime role. The short-lived admin installs both baselines only when their namespaces are
absent, owns the objects, and grants only the exact DML authority. Existing installations are
verified and never migrated, repaired, re-owned, reset, or downgraded. Managed same-crate tests run
serially through `postgres-test` against a real loopback-only `hostnossl` server and split authority.
