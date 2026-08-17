# mfm-evm-live

Direct provider registration for the three surviving EVM Read capabilities. Each callback captures
one domain-owned `EvmPhysicalTarget` and opaque provider handle, checks exact intent chain/route
before IO, bounds request encoding, and returns typed evidence or `ReadAdapterError`.

The crate owns no State registration, planner, binding wrapper, live assembly contribution, call ID,
response echo, signer, nonce, broadcast, or transaction-submission path.

Capability injection is not live registration: it is deterministic domain-owned Program topology
applied before Runtime sees the Program. This crate never invokes Operation or injection hooks.
