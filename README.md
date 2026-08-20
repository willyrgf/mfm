# MFM

MFM is a typed, append-only execution core for deterministic State/Match Programs with explicit
observational Reads. Runtime is the sole reducer, Journal seals exact canonical frames, and Store
provides only complete-prefix load and atomic exact-head append.

The current product composition is Portfolio snapshot execution over secret-free EVM balance Reads.
Application owns a typed stored-config, discovery, and run surface over one checked multi-route
composition. Callers supply an explicit `RunId`; the CLI may generate one at its client boundary.
The CLI drives config import/list/show/delete and run start/progress/show/list, documented in
[bin/cli/README.md](bin/cli/README.md). The REST binary still prints one unavailable diagnostic and
exits without binding a listener.

Start with [design](docs/design.md), [architecture](docs/architecture.md), and
[build and verification](docs/build-and-verification.md).
