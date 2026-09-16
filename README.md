# MFM

MFM is a typed, append-only execution core for deterministic State sequences with explicit
observational Reads, retained Effects, and bounded recovery. Runtime owns continuation transitions
and local validation, Journal seals exact canonical frames, and Store provides mechanical
admission/latest/optional-probe load and atomic exact-head append.

## Using MFM as a platform and framework

MFM has three public usage paths. They are activities a developer can combine, rather than
separate kinds of users:

| Activity | What the caller should express |
| --- | --- |
| Select an existing operation | Its inputs and supported options |
| Compose existing Operations or States | Their order, valid connections, and any intentional policy |
| Implement a new State | Its actual semantics and typed contracts |

An Operation packages reusable composition; a State defines one deterministic step. Reads and
Effects perform external IO through explicit adapters. All three paths use the same checked
Program and Runtime. Caller-owned choices and new semantics belong in authoring code; reusable
assembly, execution, and result handling should not be reimplemented by each caller.

These paths guide the public API, not a claim that every convenience interface already exists.
The [public interfaces and tests RFC](RFC_RESHAPING_PUBLIC_FACING_N_TESTS.md) records current
friction, proposed requirements, test responsibilities, and unresolved API decisions. Rust framework
composition and extension remain first-class uses alongside configured product entry points.

## Current products and documentation

The current product composition is Portfolio snapshots and explicit candidate-asset enrichment over
secret-free EVM balance Reads. Enrichment can publish an immutable snapshot configuration revision.
Application owns a typed stored-config, discovery, and run surface over one checked multi-route
composition. Callers supply an explicit `RunId`; the CLI may generate one at its client boundary.
The CLI drives exact-revision config import/list/delete and run start/progress/show/list, documented in
[bin/cli/README.md](bin/cli/README.md). [The REST API](bin/rest-api/README.md) serves the same typed
use cases over an unauthenticated Unix socket. The shared production bootstrap and complete
`deployment.toml` example are documented by [`mfm-app`](crates/app/README.md).

Start with [design](docs/design.md), [architecture](docs/architecture.md), and
[build and verification](docs/build-and-verification.md).
