# mfm-evm

Pure EVM domain contracts and structured operations. This crate performs no network, database,
keystore, or ambient runtime IO.

It owns:

- strict EVM routing, balance, transaction, receipt, finality, and public failure values;
- the declaration-ordered EVM balance collection operation;
- the public submission entry state and registered structured submission expansion;
- deterministic state request/settlement callbacks;
- wallet chain/domain, activation, intent, reservation, candidate, and completion contracts; and
- the narrow activation/nonce resource-authority port used by concrete storage.

Balance collection checks chain identity, captures an anchor, reads native/token values in a
bounded Read fan-out, confirms the anchor, and returns declaration-ordered results.

Submission expands into explicit status, pending-nonce, reservation, candidate attestation,
activation, broadcast, observation, reconciliation, completion, and projection states. The generic
Runtime/store never imports these types.

Signing separates `AccountAddress` (on-chain account only), full `PublicSigningIdentity` (public
key plus account), and secret signing authority (keystore / qualified read provider). Guarded
production signing requests bind the complete public identity from the qualified live signer; an
account identifier is never interpreted as public-key material. Signer integrity and binding
mismatches are classified separately from ordinary signer unavailability.

Concrete JSON-RPC and signer bindings live in `mfm-evm-live`. Real cross-run wallet authority lives
in `mfm-storage-evm-postgres`.
