# mfm-evm-live

Qualified EVM Runtime adapters and reusable one-exchange JSON-RPC transport.

The transport supports exact bounded methods needed by structured balance and submission. It has
no arbitrary method path, redirect, retry, failover, batching, scheduler, or durable lifecycle.
Response envelopes, ids, sizes, duplicate keys, quantities, hashes, receipts, and chain identity
are checked before returning reviewed domain values. Provider text and raw bodies are discarded.

Production inventory assembly starts with `PendingEvmRpcInventory::new`, which consumes the exact
private `EvmRoutingCatalog` before a transport exists. `exchange` sends the fixed bounded
qualification method over each retained endpoint and credential in catalog order and returns
provider-neutral `EvmRpcInventoryProofs`. `finish` checks the committed opaque finish authorization
and returns `CompletedEvmRpcInventoryExchange`, the only handle that proves affine continuity from
that exchange to its constructed transport. The completed handle is not deployment authority by
itself: the storage-provider/application bracket must authenticate the challenge issuer, validate
the target-held proofs, and consume the handle. Direct `EvmJsonRpcTransport::new` remains available
for reusable non-production transport use but cannot create a completed-exchange handle.

The crate registers:

- structured balance Read bindings;
- submission status/pending/lookup/finality/inclusion Read bindings;
- deterministic signer-attestation Read binding;
- exact broadcast Effect binding; and
- sealed wallet-authority Read/Effect bindings.

Each binding matches one secret-free semantic/implementation descriptor and one public physical
certificate. Live transports, signer handles, wallet clients, target sessions, and credentials are
private. The submission binding accepts only the opaque process-local
`QualifiedReadSigningProvider`, never a raw guarded provider. Qualification and every signer call
fail closed unless the provider and every transitive guard declare immutable observational
semantics: quota, approval, anti-replay, billing, rate-limit, or other externally meaningful state
consumption is ineligible for the attestation Read, with no Effect fallback. Exact broadcast
reproduces and submits one candidate once, then zeroizes bearer bytes.

Guarded signing builds the request from the qualified binding's full public key and account, verifies
signature recovery against that identity, and maps integrity/contract violations to
`IntegrityFault` rather than `SignerUnavailable`. Raw signature bytes and private-key material are
never retained in adapter evidence.

Signer implementation cutovers remain append-only physical releases. An observational keystore
`v2` release follows the retained audited `v1` certificate as a same-key-target successor, and the
complete release-history digest changes with that append.

Cross-run wallet state belongs to `mfm-storage-evm-postgres`; structured control belongs to
`mfm-evm`; Runtime owns authorization and observation persistence.
