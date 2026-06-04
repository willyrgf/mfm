# Detailed Guide

This file exists as the Nixfied discovery-friendly detailed reference for MFM.

Use these documents together:

- [`docs/design.md`](design.md) for the full runtime, persistence, replay, and secret-handling contract.
- [`docs/architecture.md`](architecture.md) for taxonomy, placement, and boundary rules.
- [`docs/PORTFOLIO_SNAPSHOT_DETAILED.md`](PORTFOLIO_SNAPSHOT_DETAILED.md) for portfolio snapshot wrapper wiring.
- [`bin/cli/README.md`](../bin/cli/README.md) and [`bin/rest-api/README.md`](../bin/rest-api/README.md) for user-facing transport contracts.

Project-owned Nixfied wiring lives in:

- [`nixfied/project/conf.nix`](../nixfied/project/conf.nix) for runtime defaults, service metadata, and discovery settings.
- [`nixfied/project/module.nix`](../nixfied/project/module.nix) for modeled tasks, workflows, and exposed app surfaces.
