# MFM

MFM is a typed append-only State runner with one byte-ingress trust boundary. The current product
supports `mfm.portfolio/snapshot@1` through its fixed-tenant App facade. Its EVM integration is
observational and supplies the balance Reads used by that Portfolio workflow.

Read [docs/design.md](docs/design.md), [docs/architecture.md](docs/architecture.md), and
[docs/build-and-verification.md](docs/build-and-verification.md) for the contracts and workflow.
