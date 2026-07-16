# mfm-app

Application assembly for catalog-backed, certified MFM runs.

`mfm-app` owns the boundary between transport input, semantic configuration, and runtime
authority. It provides:

- strict setup TOML decoding and secret-free canonicalization;
- one-transaction publication of complete typed values to the Postgres catalog;
- exact catalog reference resolution and integrity/type/semantic verification at launch;
- entry-point operation planning and typed certification;
- production store, artifact, runner, and capability wiring;
- typed start/resume/replay dispatch and public-output read authority.

The catalog is a pre-admission configuration surface. A setup document is a closed set of
supported typed values; import validates every value, canonicalizes it, rejects prohibited fields,
and appends all rows atomically. Run-start requests are strict JSON objects containing exact
`CatalogRef<T>` identities. App resolves those references before calling the operation builders and
records the resolved name/schema/digest as launch evidence. The resulting typed draft and
certified spec contain concrete values, never catalog references.

Domain runner behavior lives in adapter crates. Reusable EVM contract states are exercised by
their library-level adapter test graph; `mfm-app` does not register speculative contract runners
without a current certified public graph consumer.

After `RunAdmitted`, the run is self-contained. Resume, replay, status, stream, and public-output
reads use the certified spec, certificate, retained artifacts, and append-only run evidence. They
do not consult the mutable catalog. Public-output JSON is a cache surface and cannot authorize
resume, replay, certification, or another render.

Runtime TOML is a separate process-local routing and signer boundary. It is loaded only when a
live capability family needs it and is never part of catalog values, certified specs, events,
artifacts, or replay inputs.
