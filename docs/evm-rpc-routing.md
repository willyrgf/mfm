# EVM routing

An immutable binding descriptor records the exact State, capability, stable live-adapter
implementation, and physical route/target needed for process association and callback-free replay.
It contains no endpoint credential, deployment history, or mutable lifecycle lineage.

Trusted composition creates each descriptor and its live registration together. The adapter
rejects a wrong role, target, binding, planned balance route, or call correlation before a provider
is entered. Every current EVM operation is an observational Read used by Portfolio balance
collection.

The adapter authenticates the protocol response against the committed call, canonical intent,
capability, target, and request correlation, then discards raw provider material. Replacing a
binding rotates the Store scope or writer epoch; old nonterminal runs are replay-only.

Transaction submission is outside this routing contract. A future design must first define durable
transaction authority and outbox semantics.
