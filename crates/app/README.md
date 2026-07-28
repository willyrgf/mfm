# mfm-app

Application assembly for target-keyed, certified MFM runs.

`mfm-app` owns the boundary between transport input, semantic configuration, and runtime
authority. It provides:

- strict setup TOML decoding and secret-free canonicalization;
- one-transaction target-keyed publication of complete typed values to Postgres;
- current-target integrity/type/semantic verification at launch;
- entry-point operation planning and typed certification;
- production store-owned verified-journal, runner, and capability wiring;
- exact runtime signer/keystore assembly for canonical EIP-1559 signing;
- typed start/resume/replay dispatch and public-output read authority.

Process transports receive one opaque `Application` facade. Production construction connects one
shared Postgres store and retains it behind that facade; generic store bounds, concrete storage,
runner/certification registries, live transports, signer providers, and setup authority are not
binary concerns. Evidence-only services are initialized lazily, so readiness, facts, status, stream,
list, replay, and public-output operations never load runtime configuration. Setup operations use
the configured-value authority on the same store without live services. Every start, resume, or
manual-resolution call creates a fresh live dispatch and drops its registry and endpoint-bound
sessions when that call ends.

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

Domain live-adapter behavior lives in private adapter modules inside the live crates; ordinary pure
execution is runtime-owned.
`mfm-app` production assembly registers only the EVM balance read and atomic fact-publication
bindings required by the portfolio objective. Its
read-route validator loads selective runtime configuration on a blocking worker before admission;
no external-read ingress path performs filesystem IO on an async worker. Exact-anchor validation
and transaction submission remain separately registerable adapter/library foundations for explicit
consumers and tests; app certification, runner, and replay registries do not include them.

Live assembly hashes the opened current executable once per application process on a blocking
worker and content-addresses the exact canonical `mfm.executable-bytes.v1` identity object. The
resulting template supplies one binary digest to every distinct runner, framework, and adapter
factory. If stable executable bytes cannot be identified, startup fails with the redacted
`ExecutableIdentityUnavailable` error.
Evidence-only services never build this registry or access the executable file.

The standalone signing facade is not a second mutation workflow. It accepts raw command fields,
canonically parses and constructs the checked unsigned envelope, resolves exactly the requested
generic runtime signer and its referenced keystore, and invokes `mfm-evm`. The returned
signed envelope is transient bearer material; the app does not serialize, persist, clone, submit,
or render its bytes. The CLI uses this facade for its explicit local bearer-output command, and the
owned transaction adapter calls the same canonical `mfm-evm` primitive during transaction
preparation.

After `RunAdmitted`, the run is self-contained. The store loads and verifies one committed journal
and exposes one borrowed `VerifiedRunView` to resume, replay, status, stream, and public-output
consumers. Those consumers do not copy a stream, rebuild a projection, duplicate primary history
inside replay authority, or consult mutable current configuration. Public-output JSON is a cache
surface and cannot authorize resume, replay, certification, or another render.

Runtime TOML is a separate process-local routing and signer boundary. Each live dispatch resolves
only the distinct routes required by its certified pending nodes and fixes each selected route for
that dispatch. A later start or resume re-resolves the file and may bind a changed endpoint,
credential, timeout, or source ref. Runtime TOML is never part of current configuration, certified
specs, events, artifacts, or replay inputs.
