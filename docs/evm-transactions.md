# EVM submission

Submission identity is the fixed tenant, wallet nonce domain, and bounded non-secret idempotency
key. An upstream Read State obtains the nonce and the domain fixes a deterministic candidate before
the broadcast State prepares its Effect.

Wallet nonce reservation, candidate activation, broadcast, and completion are separate capability
Effects. Every Effect has exactly one possible provider entry; an unresolved broadcast remains
parked until its retained owner resolves or a supervisor discards it.

Provider material is authenticated and call-bound at the EVM adapter boundary. State code receives a
closed evidence value, never raw JSON-RPC bytes, signed bytes, diagnostics, or secrets. A definite
result concludes atomically; an unresolved result remains neutral.
