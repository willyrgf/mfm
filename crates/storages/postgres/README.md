# mfm-storage-postgres

Durable implementation of the two-method mechanical Store. `PostgresStore::connect` owns its pool
and gates every physical connection on the exact `mfm.run-history-postgres.v1` schema, three logged
tables, primary status, `fsync=on`, and `full_page_writes=on`.

Loads use one read-only repeatable-read snapshot, prove a nonempty gap-free prefix and byte/digest
accounting, then copy owned rows in one pure blocking job. Appends copy the candidate before BEGIN,
force `synchronous_commit=on`, take the per-RunId advisory transaction lock before state reads,
validate head/target, insert immutable bytes, update the head, and COMMIT.

Pre-COMMIT failures are definite typed capacity/corruption/unavailability. Only an IO/protocol loss
after COMMIT submission is `Indeterminate`. Static schema installation is the migration artifact,
not a public Store method. Managed same-crate tests run through the `postgres-test` task.
