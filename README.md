# MFM

MFM is a typed, append-only execution core for deterministic State/Match Programs with explicit
observational Reads. Runtime is the sole reducer, Journal seals exact canonical frames, and Store
provides only complete-prefix load and atomic exact-head append.

The current product composition is Portfolio snapshot execution over secret-free EVM balance Reads.
Callers supply an explicit `RunId`; Application is a thin facade over one already-composed Runtime.
The CLI exposes one-shot help/version metadata. The REST binary prints one unavailable diagnostic
and exits without binding a listener.

Start with [design](docs/design.md), [architecture](docs/architecture.md), and
[build and verification](docs/build-and-verification.md).
