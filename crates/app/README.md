# mfm-app

Application assembly for target-keyed, certified MFM runs.

`mfm-app` owns the boundary between transport input, semantic configuration, and runtime
authority. It provides:

- strict setup TOML decoding and secret-free canonicalization;
- one-transaction target-keyed publication of complete typed values to Postgres;
- current-target integrity/type/semantic verification at launch;
- entry-point operation planning and typed certification;
- production store, artifact, runner, and capability wiring;
- exact runtime signer/keystore assembly for canonical EIP-1559 signing;
- typed start/resume/replay dispatch and public-output read authority.

Current configuration is a pre-admission surface. A setup document is a closed set of supported
typed values; import validates every value, derives its stable target from the intrinsic domain id,
canonicalizes it, rejects prohibited fields, and atomically creates, updates, or leaves unchanged
the current row for each target. Run start supplies an exact entry-point id plus a target. App
resolves that current row before calling operation builders and records target/schema/digest as
launch evidence. The resulting typed draft and certified spec contain concrete values.

The sole public objective is `mfm.portfolio/snapshot@1`. Its REST request has exactly these fields:

```json
{
  "entry_point": "mfm.portfolio/snapshot@1",
  "target": "acme/primary"
}
```

The request carries no collector policy, child config, runtime route, or report-only/reuse mode.
Setup publishes only `PortfolioConfig` in this domain. The app resolves and normalizes that value
once at admission, records its target/schema/digest in `RunAdmitted`, and passes the concrete value to
`PortfolioSnapshotOperation`. That operation calls the required family collectors and one
`PortfolioReportOperation`; it constructs no state directly. Resume, replay, status, stream, and
public-output reads never consult current configuration.

Domain runner behavior lives in adapter crates. `mfm-app` registers the shared reusable EVM read
assembly for balance collection and exact-anchor validation, plus the reusable EVM transaction
binding. The shared read-route validator loads selective runtime configuration on a blocking worker
before admission; no external-read ingress path performs filesystem IO on an async worker. It also
registers the EVM collector operation/fact descriptors needed by composed and internal runs. Public
discovery remains the single certified portfolio objective; the internal EVM collector cycle has no
app target, resolver, renderer, or discovery id.

The standalone signing facade is not a second mutation workflow. It accepts raw command fields,
canonically parses and constructs the checked unsigned envelope, resolves exactly the requested
generic runtime signer and its referenced keystore, and invokes `mfm-evm-signing`. The returned
signed envelope is transient bearer material; the app does not serialize, persist, clone, submit,
or render its bytes. The CLI uses this facade for its explicit local bearer-output command, and the
owned transaction adapter calls the same canonical `mfm-evm-signing` primitive during transaction
preparation.

After `RunAdmitted`, the run is self-contained. Resume, replay, status, stream, and public-output
reads use the certified spec, certificate, retained artifacts, and append-only run evidence. They
do not consult mutable current configuration. Public-output JSON is a cache surface and cannot
authorize
resume, replay, certification, or another render.

Runtime TOML is a separate process-local routing and signer boundary. It is loaded only when a
live capability family needs it and is never part of current configuration, certified specs, events,
artifacts, or replay inputs.
