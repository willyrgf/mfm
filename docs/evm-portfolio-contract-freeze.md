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
schema:mfm.evm-physical-target:1:sha256-jcs-v1:4f21dfcf2cbe47e513963fff2d6da4e73c7f603dcaf5fb558b670f27f82cab32
content:sha256-v1:7701e82ec5bd36b79b7e361b85bfe358494b531ad687b2683a397eaf7f44037e
```

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

EVM transaction submission has no entry point, Program, capability, nonce/signer/broadcast
composition, or fixture. Reintroduction requires a separate durable transaction-authority/outbox
RFC.
