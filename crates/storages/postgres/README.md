# mfm-storage-postgres

Durable implementation of the two-method mechanical Store. `PostgresStore::connect` owns its pool
and gates every physical connection on the exact `mfm.run-history-postgres.v1` schema, three logged
tables, primary status, `fsync=on`, and `full_page_writes=on`.

Loads use one read-only repeatable-read snapshot, prove a nonempty gap-free prefix and byte/digest
accounting, then copy owned rows in one pure blocking job. Appends copy the candidate before BEGIN,
force `synchronous_commit=on`, take the per-RunId advisory transaction lock before state reads,
validate head/target, insert immutable bytes, update the head, and COMMIT.

Pre-COMMIT failures are definite typed capacity/corruption/unavailability. Only an IO/protocol loss
after COMMIT submission is `Indeterminate`.

`install_schema` is the one public provisioning entry and is not a Store method: Store-trait rules
constrain trait semantics, not physical self-provisioning. It verifies an installed schema and
returns `Ok`, executes the static migration only when the durability posture already holds and the
public schema owns no `mfm_`-prefixed relation at all, and otherwise returns `Incompatible` without
touching one byte. It never migrates, repairs, or downgrades. Managed same-crate tests run through
the `postgres-test` task, serially, because each owns the whole managed database.
