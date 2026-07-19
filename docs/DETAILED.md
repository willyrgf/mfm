# Detailed Guide

This file exists as the Nixfied discovery-friendly detailed reference for MFM.

Use these documents together:

- [`docs/design.md`](design.md) for the full runtime, persistence, replay, and secret-handling contract.
- [`docs/architecture.md`](architecture.md) for taxonomy, placement, and boundary rules.
- [`docs/portfolio-snapshot.md`](portfolio-snapshot.md) for the sole public portfolio workflow.
- [`docs/btc-rpc-routing.md`](btc-rpc-routing.md) for Bitcoin runtime routing and strict snapshot semantics.
- [`docs/evm-rpc-routing.md`](evm-rpc-routing.md) for bounded source-bound EVM runtime sessions.
- [`docs/evm-transactions.md`](evm-transactions.md) for EVM mutation, recovery, and validation composition.
- [`docs/persisted-public-surfaces.md`](persisted-public-surfaces.md) for persisted authority and secret boundaries.
- [`bin/cli/README.md`](../bin/cli/README.md) and [`bin/rest-api/README.md`](../bin/rest-api/README.md) for user-facing transport contracts.

Project-owned Nixfied v2 wiring lives in:

- [`flake.nix`](../flake.nix) for the Nixfied input pin and exposed app wrappers.
- [`nixfied.nix`](../nixfied.nix) for modeled tasks, composites, managed services, slots, and ports.
