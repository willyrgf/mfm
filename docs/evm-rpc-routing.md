# EVM routing

An immutable binding descriptor records the exact State, capability, adapter implementation,
physical route/target, Effect domain, and public signer key-instance identity needed for process
association and callback-free replay. It contains no endpoint credential, deployment history,
mutable lifecycle lineage, or private key.

The adapter authenticates the protocol response against the committed call, canonical intent,
capability, target, and request correlation, then discards raw provider material. Replacing a
binding rotates the Store scope or writer epoch; old nonterminal runs are replay-only.
