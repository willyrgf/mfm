# EVM Read and Portfolio contract freeze

Status: current binding and public-result contract.

## Material uncertainties

none.

The three canonical public/domain-handoff fixtures in `docs/contracts/evm-portfolio/` remain the
source of truth:

- `evm-balance-collection.json` is the typed EVM-to-Portfolio completion;
- `portfolio-snapshot.json` is the public success projection; and
- `portfolio-snapshot-failure.json` is the public failure projection.

They use canonical JSON, decimal strings, declaration order, and no floats. Runtime Program and
Journal wires are intentionally not these public result fixtures.

The physical routing value is exactly semantic `mfm.evm/physical-target@1`, schema
`mfm.evm-physical-target@1`, and canonical fields `chain_id` plus `endpoint_ref`. For the test target
with chain ID 1 and fixed endpoint bytes, the frozen instance ref is:

```text
schema:mfm.evm-physical-target:1:sha256-jcs-v1:2ee09931b3289c2a2169788badc06101af145d6628751550bbd21d30d6ac346c
content:sha256-v1:7701e82ec5bd36b79b7e361b85bfe358494b531ad687b2683a397eaf7f44037e
```

The schema ID reflects the current-only EVM primitive cut: `chain_id` declares its exact nonzero
`u64` range, balance sources use the shared checked `EvmAddress`, and block anchors use `EvmU256`
plus `EvmHash`. Valid canonical portfolio result JSON remains byte-identical.

Portfolio owns selector validation, target selection, collection ordinal/correlation, route
retention, child failure mapping, quote selection, and final snapshot/report projection. EVM owns
chain/anchor/source validation, raw quantities and decimal scale, common-anchor confirmation, and
collection completion. Live EVM owns only bounded provider ingress for the exact planned Read.

Portfolio authors the root through a private Operation and composes the public configured
`CollectEvmBalances<K>` child without forecasting its size. EVM owns the child topology and six
explicit capability/State binding policies. Operation and injection metadata are not part of this
frozen Program or result wire.

`Rejected` and `SafeFailure` map through ordinary source-failure interpretation.
`IntegrityBlocked` remains distinct through the EVM failure and Portfolio mapper. A local target or
route mismatch is Runtime `Internal`, never manufactured durable evidence.

The generic Effect protocol, EVM transaction contracts, append-only nonce/raw/settlement authority,
live transaction adapter, and first-party contract fixture now exist as reusable lower-level
contracts. EVM owns the public action-specific creation/call projections and checked anchored
transitions. The managed fixture still owns its exact lifecycle topology, ABI, policy, Operation,
report, and registrations locally and proves two mutations plus an anchored call; none of that
topology is part of Portfolio.

Portfolio still exposes no transaction entry point, capability use, signer/authority composition,
contract lifecycle, or mutation result. Production `ComposedRuntime`, configuration, CLI, and REST
register only the existing observational Portfolio Reads. Adding mutation to Portfolio would require
a separately reviewed product and production-finality contract; this fixture does not imply one.
