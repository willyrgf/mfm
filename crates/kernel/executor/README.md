# MFM executor

`mfm-executor` owns the domain-free keyed-convergence contract used for recoverable external
effects. It provides:

- immutable, content-addressed executor binding and deployment identities;
- strict executor-contract descriptors bound to exactly one verified binding;
- an append-only effect and resource ledger contract;
- bounded predecessor-linked delivery-audit frontiers;
- affine target-entry authorization whose private completion seal normalizes and binds an opaque
  target outcome before immutable observation;
- terminal tombstones;
- a memory conformance backend;
- account-sequence and finite-inventory resource-policy examples; and
- a convergence-safe durable-queue reference executor.

The crate does not own MFM run state, scheduling, retries, credentials, stateless protocol
transports, or domain-specific settlement. Its memory backend and destination are qualification
components, not a production disaster-recovery authority. A production backend must additionally
provide non-rollback persistence and an authoritative destination fence.

`KeyedExecutorLedger<Store>` is the one storage-neutral high-level engine. It owns effect folding,
typed resource-policy evaluation and refold, deterministic attempt derivation, bounded local CAS
retry, observation, and terminal semantics. `ExecutorLedgerStore` is deliberately narrower: an
asynchronous backend exposes its exact fenced identity, loads complete immutable effect and resource
histories and exact content objects, and atomically compares their current heads while appending one
complete proposal. Backends do not reimplement executor policy.

`ExecutorAppendOutcome::Applied` is the only outcome that can mint affine target-entry authority.
`AlreadyApplied` and `Conflict` cause a fresh load and derivation; `OutcomeUnknown` returns a closed
error and never recreates authority. Bind plus first allocation, or the allocation upgrade of an
already-bound effect with no attempts, commits the effect frontier and resource record atomically.
Account-sequence allocation cannot advance past an allocated effect until that effect has immutable
terminal evidence. Locks and transactions are released before target IO.

Target authorization accepts the complete schema-qualified target-entry descriptor, derives its
content reference, and retains the descriptor in the same atomic proposal as the authorization.
Recovery resolves that exact descriptor by attempt identity before choosing another target entry;
a lightweight family reference alone is insufficient. Domain descriptors may retain only reviewed
public target inputs and commitments. Signatures, signed envelopes, credentials, and other bearer
material remain below the target boundary.

Reopen validates exact store identity and strictly refolds the complete immutable effect/resource
graph and executor-owned content inventory. Missing, extra, duplicate, mismatched, forked, or
partially linked content fails closed. `ReferenceDestination` is the separate convergence boundary
and must enforce generation fencing and semantic-key idempotency across every process.

Requests use `SchemaQualifiedCanonicalValue`, which accepts arbitrary certified domain schemas and
retains exact float-free canonical bytes. `ExecutorContractDescriptor` binds the semantic request,
safe failure, and every retained closure relation to the one shared
`mfm_values::RetainedValueContract`: schema, semantic type, role, media type, and evidence
contract. Domain values remain reachable through the shared producer-bound `mfm.value-ref.v1`
journal object; this crate defines no substitute `ValueRef`. A `CommittedEffectRequest` fixes data
identity but grants no target access.
`TargetEntryAuthority` exposes only the four durable entry coordinates needed by a target. The
ledger retains the corresponding affine completion seal, normalizes the opaque target outcome, and
binds it to those coordinates before appending its observation.

Resource allocation retains and revalidates the exact policy and policy-configuration reference
pair on every refold. `verify_ensure_result` reconstructs one predecessor-linked delivery audit,
tombstone, frozen generic v1 exact-attempt proof, and domain evidence only from an
`ExecutorRetainedClosureClaim`. It rejects missing members, extra or forked frontiers, unreachable
objects, relation substitution, contract substitution, and a different executor binding.
`VerifiedEnsureResult::retained_closure` exposes the resulting producer-free
`VerifiedExecutorRetainedClosure`. The head frontier is the `DeliveryAudit` relation; predecessor
frontiers, tombstone, proof, and domain evidence each have their own closed relation and
descriptor-selected contract.

The executor never constructs the journal's outer ensure-result or terminal-evidence value and
never assigns producer authority. Runtime deterministically constructs those two values using the
same descriptor's `ensure_result_contract` and `terminal_evidence_contract`, then performs the
sole whole-graph promotion under the already committed external-observation authorization. The
effect callback separately decides domain finality.

`MemoryLedgerCheckpoint` and `MemoryDestinationCheckpoint` provide bounded checksummed restart
encoding. The bytes have integrity checks but no independent persistence, freshness, or
anti-rollback authority. A PostgreSQL backend must store immutable records as authority, use
rebuildable CAS heads only as indexes, reject corrupt or stale generations without ancestor
fallback, and combine restore with an authority outside any rollback domain.
