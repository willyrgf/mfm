# EVM JSON-RPC Routing

Status: current structured Runtime routing contract

## Material uncertainties

none

## Boundary

EVM JSON-RPC transports perform one bounded protocol exchange. They do not inspect run history,
choose structured control, retry invisibly, allocate nonces, settle states, or persist evidence.
Runtime-authorized live adapters bind an admitted request to one qualified transport route and
convert every surviving return into a reviewed observation outcome.

## Qualified route identity

Qualified deployment infrastructure is the sole issuer of chain-instance and route-membership
authority. A complete secret-free catalog contains the provider-issued chain attestations, every
route generation and membership issuance, the qualified registry lineage, and the registry head at
qualification. Application assembly accepts it only after an authenticated provider has checked
that exact catalog against its pinned lineage and exact current head and returned an opaque,
non-serializable `QualifiedEvmRoutingCatalog`. Public descriptor bytes, even when structurally
valid, cannot construct that value.

Admission derives its immutable routing policy from the provider-qualified catalog. A route
identifies its full chain-instance attestation, chain id, routing generation, membership issuance,
implementation, and public physical certificate. The admitted policy, live route, portfolio
binding, wallet activation, transaction intent, and submission request must carry the same full
chain binding; numeric chain-id equality cannot substitute. Cross-chain or foreign-membership
combinations are rejected before admission.

Secrets and endpoint text remain in process-private transport construction; they are absent from
certification and history. Cloning the provider client copies only endpoint configuration and a
public trust anchor. Provider signing authority and target inventory remain in the separate
provider process.

The store sees only a purpose-limited public certificate and, for refreshable Effects, public
lineage-head/evidence objects. Those values are evidence, not target sessions or mutation permits.
Live registration supplies an exact purpose tuple binding the capability and qualified adapter
implementation to an ordered immutable history of physical target, full certificate, and admitted
routing-policy releases. The store accepts any retained release only while callback-free refolding
recorded history; a new authorization must use the current exact release. Refreshable Effects also
bind every successor to its predecessor certificate and activation lineage head, so supersession
proves a strict old-to-new relation without giving the store a client, pool, signer, or fence
issuer.

## One exchange

The reusable transport:

1. validates the typed request and exact method/parameter contract;
2. writes one bounded JSON-RPC exchange to the selected route;
3. validates response envelope, request id, size, and method-specific result;
4. returns a reviewed typed value or redaction-safe transport classification; and
5. discards raw bodies and provider text.

No provider-supplied error message, response body, URL, header, credential, or debug source is
retained or rendered. Public errors use reviewed fixed codes/messages.

## Read behavior

Structured balance and submission Reads include chain identity, latest/finalized anchor, native
balance, token decimals/balance, anchor confirmation, pending nonce, transaction lookup, receipt,
and canonical-inclusion lookup. Each authorization permits one exchange. A stale Read binding maps
to its reviewed `SafeFailure`; it does not create a physical refresh attempt. An unmatched Read
waits.

## Effect behavior

Exact transaction broadcast is an Effect. Authorization binds the exact request digest, physical
certificate, stable broadcast-resource lineage, semantic head, occurrence path, and attempt
ordinal before possible entry.

- If revocation/non-entry is purpose-limited and provable, the invoker returns
  `SupersededBeforeEntry`; Runtime may authorize the next ordinal.
- If target entry may have occurred, it returns `EntryUnknown`; the occurrence parks and cannot
  submit again automatically.
- A definite reviewed destination response becomes `Returned` or `SafeFailure` according to the
  certified capability contract.
- Malformed or inconsistent evidence becomes integrity handling and never state truth.

The exact broadcast adapter signs/reproduces one deterministic envelope, submits it once, and
zeroizes bearer bytes. A route switch cannot alter candidate identity.

## Chain-instance identity

Chain id and genesis alone are insufficient. Qualified EVM resource domains additionally bind a
never-reused instance namespace and finalized fork anchor. Multiple redundant routes can be
members of one declared instance. Independently operated forks, restored clones, or aliases cannot
share one wallet nonce domain.

## Routing changes

Reads can use a newly assembled provider-qualified route on a later run. Effect rotation for an
outstanding occurrence requires committed pre-entry supersession evidence. Wallet target/session
promotion is a separate resource-authority procedure described in `docs/evm-transactions.md`;
changing a public route certificate cannot mint a wallet mutation permit. Normal wallet execution
uses its immutable provider-qualified activation proof with offline verification and performs zero
chain-registry queries. A fresh process must be assembled with the complete append-only release
history: it replays older admitted certificates as retained evidence, selects only the current
route/certificate for new appends, and verifies the recorded predecessor-to-successor edge before
continuing an outstanding Effect.

## Tests

Transport tests cover exact request bytes, response id/result validation, bounds, unknown fields,
provider-text redaction, and Returned/SafeFailure mapping. Structured Runtime tests cover affine
invocation and refresh rules. The managed submission qualification uses a loopback JSON-RPC server
and asserts one exact broadcast across process restart.
