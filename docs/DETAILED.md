# Detailed Guide

This file exists as the Nixfied discovery-friendly detailed reference for MFM.

Use these documents together:

- [`docs/redesign.md`](redesign.md) for the full runtime, persistence, replay, and secret-handling contract.
- [`docs/ops-and-states.md`](ops-and-states.md) for the current op and state inventory.
- [`docs/helios.md`](helios.md) for Helios-backed snapshot wiring and service checks.
- [`bin/cli/README.md`](../bin/cli/README.md) and [`bin/rest-api/README.md`](../bin/rest-api/README.md) for user-facing transport contracts.

Project-owned Nixfied wiring lives in:

- [`nixfied/project/conf.nix`](../nixfied/project/conf.nix) for runtime defaults, service metadata, and discovery settings.
- [`nixfied/project/module.nix`](../nixfied/project/module.nix) for modeled tasks, workflows, and exposed app surfaces.
