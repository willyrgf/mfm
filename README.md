# MFM

MFM is a typed, append-only execution core for deterministic State/Match Programs with explicit
observational Reads. Runtime is the sole reducer, Journal seals exact canonical frames, and Store
provides only complete-prefix load and atomic exact-head append.

The current product composition is Portfolio snapshot execution over secret-free EVM balance Reads.
Application owns a typed stored-config, discovery, and run surface over one checked multi-route
composition. Callers supply an explicit `RunId`; the CLI may generate one at its client boundary.
The CLI drives exact-revision config import/list/delete and run start/progress/show/list, documented in
[bin/cli/README.md](bin/cli/README.md). [The REST API](bin/rest-api/README.md) serves the same typed
use cases over an unauthenticated Unix socket. The shared production bootstrap and complete
`deployment.toml` example are documented by [`mfm-app`](crates/app/README.md).

Start with [design](docs/design.md), [architecture](docs/architecture.md), and
[build and verification](docs/build-and-verification.md).
