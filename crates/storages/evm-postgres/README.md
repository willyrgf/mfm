# mfm-storage-evm-postgres

Real PostgreSQL activation registry and wallet nonce authority for structured EVM submission.

The domain port and canonical request/proof types live in `mfm-evm`. This crate owns SQL,
role-specific private pools, the provider protocol that authenticates the chain-registry lineage
and route-membership catalog at a pinned non-rollback head, target/session enforcement, permanent
operation keys, and linearizable status/reserve/activate/complete transactions. It performs no
JSON-RPC, signing, Runtime scheduling, or RunHistory folding.

`migrations/0001_wallet_authority.sql` is the one current schema. It creates separately privileged
owner, activation-registry admin/public, and wallet-application surfaces. Normal status/mutation
uses an immutable activation proof with offline verification and makes zero registry queries.

Each runtime surface authenticates through its own unprivileged `LOGIN NOINHERIT` principal. The
principal must hold exactly one of `mfm_evm_wallet_activation_admin`,
`mfm_evm_wallet_activation_public`, or `mfm_evm_wallet_nonce_application`, with permission to
activate only that role. Managed tasks supply those credentials through
`MFM_EVM_WALLET_ACTIVATION_ADMIN_DATABASE_URL`,
`MFM_EVM_WALLET_ACTIVATION_PUBLIC_DATABASE_URL`, and
`MFM_EVM_WALLET_NONCE_APPLICATION_DATABASE_URL`; `DATABASE_URL` remains the migration-owner
boundary. Every pool open verifies the authenticated and active identities, the complete role
membership graph, and the closed schema/table/function ACLs before use. Role-specific constructors
reset any caller-supplied active role before selecting their one expected role; the separate
provider process requests that same activation-admin role explicitly at connection startup.
The catalog manifests are generated against the pinned PostgreSQL 18 verification service; a
PostgreSQL major-version change requires a deliberate manifest regeneration and qualification.

Every status read requires a sealed current target session. Every mutation additionally consumes a
fresh non-cloneable permit bound to the physical target, database session, transaction, store
lineage, and writer epoch. The concrete provider is supplied by qualified deployment
infrastructure through a distinct process; public wire evidence cannot mint a session or permit.

The provider returns an opaque non-serializable `QualifiedEvmRoutingCatalog`; public registry,
attestation, and membership objects alone cannot construct it. Cloning the client copies only its
endpoint configuration and public trust anchor. Provider signing authority and target inventory
remain in the separate provider process. Normal wallet execution verifies the immutable activation
proof offline and makes zero registry queries. The activation issuance reference commits the
logical provider identity and activation record; restart hydration recomputes it, so only the same
logical provider can recover issued activations.

Deployment assembly is an affine Begin/Finish bracket. Revocation atomically closes Begin and
invalidates every pending assembly lease, and Finish succeeds only when its lease and all bound
catalog, fence, wallet, route, and endpoint proofs are still current before that cutover. Reported
and drained lease totals include both ordinary authority leases and nonexpired assembly leases.

Provider children are restartable enforcement processes, not checkpoint owners. A separate
external checkpoint authority retains the exact target, fence lineage, epoch, acknowledged SQL
prefix, and optional `Prepared` transition. Mutations prepare the exact predecessor/successor
before SQL and acknowledge only after observing that successor. On startup, a prepared
predecessor remains prepared and accepts only the identical retry; a prepared successor is
finalized. Rollback, database-ahead state, a sibling successor, copied target, or mismatched
incarnation/public head rejects readiness without compatibility recovery.

The authority enforces permanent domain activation, first/later fresh pending-nonce rules, one
incomplete stable intent, monotonic high water, a bounded candidate family, contiguous activation,
older-candidate completion, canonical terminal convergence, and acknowledgement-safe permanent
operation keys.

Run managed qualification with:

```sh
nix run .#run -- --task wallet-nonce-postgres-qualification
```

It exercises real SQL, restricted roles, process restart, contention, rollback, copied targets,
stale sessions, replayed permits, activation/promotion constraints, and end-to-end structured EVM
submission. Its deployment-assembly matrix also covers registry-head advance, stale Begin and
Finish, duplicate lease replay without overwrite, expiry, disconnect, revoke-between-Begin/Finish,
crash/restart, cross-provider activation reuse, exact successful-Finish accounting, and absence of
partial nonce-authority persistence.
