# EVM routing

An immutable binding descriptor records the exact State, capability, stable live-adapter
implementation, physical route/target, Effect domain, and public signer key-instance identity
needed for process association and callback-free replay. It contains no endpoint credential,
deployment history, mutable lifecycle lineage, or private key.

Trusted composition creates the descriptor and its live registration together. Reads carry neither
effect domain nor signer; nonce reservation uses the tenant/sender/nonce-domain-derived effect
domain; broadcast requires both an Effect domain and the exact public signer key instance. The
adapter rejects a wrong role, target, binding, planned balance route, signer, effect domain,
or call correlation before a provider, signer, or nonce authority is entered.

The adapter authenticates the protocol response against the committed call, canonical intent,
capability, target, and request correlation, then discards raw provider material. Replacing a
binding rotates the Store scope or writer epoch; old nonterminal runs are replay-only.
