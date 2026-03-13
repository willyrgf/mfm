# mfm-state-aave-v3

Reusable Aave V3 deploy/configure and portfolio-position states for MFM workflows.

This crate holds shared execution states used by Aave-focused ops and integrations.

Current reusable surfaces:

- deploy/configure/adaptation flow states under `src/states.rs`
- canonical `protocol_position` portfolio config validation under `src/portfolio/model.rs`
- canonical Aave reserve/debt observation collection under `src/portfolio/states.rs`
