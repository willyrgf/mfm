# mfm-storage-postgres

`PostgresBackend` implements the append-only Store, mechanical RunIndex, and opaque versioned
configuration repository. Optional `PostgresEvmTransactionAuthority` implements only the EVM
transaction-authority port. Each owns an independent pool and exact connection gate for its own
schemas on the same normalized PostgreSQL target.

Production constructors accept only one bounded private `postgresql` URI with an explicit password,
numeric `127.0.0.1` or `::1` host, and `sslmode=disable`. PostgreSQL is
intentionally plaintext inside the trusted shared network namespace and no remote database target
is supported. The parser uses stock SQLx, overwrites every ambient-derived connection value that
can affect this plaintext connection, and rejects `PGOPTIONS`, whose startup effects SQLx cannot
clear. Home/passfile and service-file inputs cannot influence the resulting authority. SQLx remains
an unmodified crates.io dependency. All static production SQL uses its checked query macros.
Every runtime connection must authenticate as the fixed `mfm_runtime` role and pass the exact role,
ownership, database/schema, durability, and declarative catalog gate. One normalized verifier owns
all three surfaces and compares expanded schema, relation, and column ACLs (including `MAINTAIN`),
column storage shape, RLS/policies/rules/triggers, relation storage properties, validated
constraints, and live/ready/valid indexes. Dedicated schemas are closed to unmodeled relations;
the public run-history surface admits unrelated application relations but checks every object
attached to its three named tables.
The supported server major is PostgreSQL 18, pinned by the managed environment; another major must
update and requalify the catalog specification before it is admitted.

Loads use one read-only repeatable-read snapshot, prove a nonempty gap-free prefix and byte/digest
accounting, then copy owned rows in one pure blocking job. Appends copy the candidate before BEGIN,
force `synchronous_commit=on`, take the per-RunId advisory transaction lock before state reads,
validate head/target, insert immutable bytes, update the head, and COMMIT.

Pre-COMMIT failures are definite typed capacity/corruption/unavailability. Only an IO/protocol loss
after COMMIT submission is `Indeterminate`.

Configuration import relies on the `(name, digest)` primary key to create or compare immutable
bytes, forces synchronous COMMIT, and distinguishes definite failure from ambiguous
acknowledgement. Exact delete is idempotent and uses the same acknowledgement contract. Listing
returns every revision in ascending bytewise name/digest order; documents remain individually
bounded at 256 KiB, but the revision collection has no count limit.

The `mfm_evm_tx` v1 schema has exactly its marker, nonce reservations, exact prepared transactions,
and canonical typed settlements. A reservation contains the complete nonce domain; there is no
separate domain relation. Its generated 32-byte epoch distinguishes every fresh installation.
Reservation uses an endpoint-independent domain advisory lock and exact `NUMERIC(20,0)` u64
conversion.
One marker-driven statement qualifies the captured epoch and reconstructs the optional complete
nested state without hiding a wrong-epoch reservation. Construction captures the direct-gate epoch,
and every pooled physical connection repeats the full gate and requires that same epoch. Every write
uses synchronous COMMIT; an ambiguous acknowledgement or a different concurrent Prepared/Settled
winner is `Unavailable`, and the next caller-driven `load` resolves it.

`provision_postgres` installs or verifies only run-history and configuration custody;
`provision_evm_transaction_authority` independently installs or verifies the optional authority.
Both require distinct typed admin/runtime locators for the same normalized target and an
already-created fixed runtime role. Production CLI provisioning invokes only the base provisioner. Existing
installations are verified and never migrated, repaired, re-owned, reset, or downgraded. Managed
same-crate tests run serially through `postgres-test` against a real loopback-only `hostnossl`
server and split authority, including hostile catalog and ACL mutations that every independent pool
must reject.

PostgreSQL data reads and deletes target schema-qualified `ONLY` relations. Inserts and conflict
updates target the named physical table. Inherited descendants outside the owned schemas are
outside MFM custody: their rows cannot affect markers, history, configuration, or transaction
authority, and exact configuration deletion cannot remove their rows. Admission continues to
qualify the owned objects; it does not police unrelated inheritance descendants.

Static queries compile against the committed workspace `.sqlx` metadata. Ordinary development,
verification, and packaging default to offline compilation. `sqlx-prepare` rebuilds metadata in a
fresh disposable PostgreSQL 18 database from the three SQL baselines; `sqlx-check` independently
rebuilds it and rejects missing, changed, or extra cache entries. No production credentials or
provisioning code participate in metadata generation. See `docs/build-and-verification.md` for
commands. Compile-time checks cover SQL names and types; runtime admission still qualifies the
actual deployment, and domain/canonical validation still qualifies stored values.

The only unchecked production SQL is the three existing multi-statement installation baselines
and database ACL statements whose identifier is quoted by PostgreSQL `format('%I', ...)`. Hostile
and fault-injection test SQL remains dynamic. Store frame-bearing queries use checked macros
through `Executor` to retain raw rows: their byte copies and physical validation remain inside the
pure blocking job. The append job constructs checked, owned query arguments; only the async caller
executes the query. Catalog nullability overrides follow the selected expression: outer joins and
optional expressions remain nullable; required catalog facts and total expressions are non-null.
