# EVM submission

Submission identity is the fixed tenant, wallet nonce domain, and bounded non-secret idempotency
key. Nonce reservation is one durable one-entry Effect, not a Read: its exact effect domain is
derived from tenant, sender, and nonce domain. Only its durable reservation evidence permits the
pure candidate derivation to construct the deterministic candidate and complete broadcast intent.

Broadcast is the second and final Effect. Its committed intent fixes chain/target, sender, nonce
domain, nonce, idempotency key, candidate id, data, gas, fees, and public signer key-instance
identity before signing or provider entry. Every Effect has exactly one possible provider entry; an
acknowledgement-unknown reservation or broadcast remains parked and never creates a second nonce or
candidate.

A broadcast hash is not an inclusion claim. The final Program suffix is receipt Read, finalized-head
Read, canonical-inclusion Read, then pure disposition consolidation. The public
`succeeded`/`reverted` disposition exists only when the receipt hash and block, finalized head, and
canonical block all agree. Any unavailable or inconsistent evidence follows the declared typed
failure route and suppresses later normal work.

Provider material is authenticated and call-bound at the EVM adapter boundary. State code receives a
closed evidence value, never raw JSON-RPC bytes, signed bytes, diagnostics, or secrets. A definite
result concludes atomically; an unresolved result remains neutral.
