# MFM PostgreSQL executor storage

`mfm-storage-executor-postgres` is the production storage implementation for the raw
`mfm-executor` ledger contract. It is not part of the MFM run-journal store and cannot borrow that
store's writer authority.

The executor ledger must use a dedicated PostgreSQL schema, migration owner, and
`mfm_executor_application` role. Its binding, effect-frontier, resource, effect/resource-link, and
content rows are immutable authority. Effect and resource heads are derived views over those rows;
there is no separately mutable or authoritative head row.

Opening the store consumes a deployment implementation of the executor-specific writer-generation
fence. The fence must prove the exact tenant, executor binding, durable ledger generation,
database, and schema, and must permanently exclude stale and sibling writers across restore,
rollback, and promotion. There is no default fence, test bypass in the production API, unfenced
constructor, replica mode, or adapter from the run-store fence.

The backend performs only raw asynchronous loads and compare-and-append. The shared
`mfm-executor` engine authors proposals and owns every fold, resource-policy decision, evidence
bound, and target-entry rule. A compare-and-append returns `Applied`, `AlreadyApplied`, `Conflict`,
or `OutcomeUnknown`; only the shared engine may turn a directly observed `Applied` authorization
append into fresh affine target-entry authority.

Transactions take locks in the fixed order ledger generation, effect key, then optional resource
key. A pre-bound effect's allocation record and exact resource link commit atomically. Every
transaction ends before destination, signer, wallet, or RPC work begins. Readiness checks only the
bounded ledger identity/schema contract and the held deployment fence.

The schema admits one immutable executor binding and therefore one tenant and resource-ownership
domain. A domain policy such as EVM account sequence must receive its exact deployment-attested
initial value before proposing the first resource record; this storage crate never observes a
pending nonce, chooses a default, opens a signer, or infers wallet ownership.
