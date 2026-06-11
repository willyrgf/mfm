# Detailed Guide

This file exists as the Nixfied discovery-friendly detailed reference for MFM.

Use these documents together:

- [`docs/design.md`](design.md) for the full runtime, persistence, replay, and secret-handling contract.
- [`docs/architecture.md`](architecture.md) for taxonomy, placement, and boundary rules.
- [`docs/PORTFOLIO_SNAPSHOT_DETAILED.md`](PORTFOLIO_SNAPSHOT_DETAILED.md) for portfolio snapshot wrapper wiring.
- [`bin/cli/README.md`](../bin/cli/README.md) and [`bin/rest-api/README.md`](../bin/rest-api/README.md) for user-facing transport contracts.

Project-owned Nixfied v2 wiring lives in:

- [`flake.nix`](../flake.nix) for the Nixfied input pin and exposed app wrappers.
- [`nixfied.nix`](../nixfied.nix) for modeled tasks, workflows, managed Postgres/Reth services, slots, and ports.
