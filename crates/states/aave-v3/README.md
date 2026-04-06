# mfm-state-aave-v3

Reusable Aave V3 portfolio-position types and runtime adapters for MFM workflows.

This crate holds the shared Aave domain surface used by the generic portfolio tracker.

Current reusable surfaces:

- canonical `protocol_position` portfolio config validation under `src/portfolio/model.rs`
- canonical Aave reserve/debt observation payloads under `src/portfolio/plan_payloads.rs`
- canonical Aave reserve/debt runtime adapters under `src/portfolio/plan_adapters.rs`
