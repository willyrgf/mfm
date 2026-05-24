# mfm-state-aave-v3

Reusable Aave V3 portfolio-position runtime adapters for MFM workflows.

Stable Aave semantic config lives in `mfm-portfolio-model`; compiled payload contracts live in
`mfm-portfolio-plan`. This crate holds the runtime adapter surface used by the generic portfolio
tracker.

Current reusable surfaces:

- canonical Aave reserve/debt runtime adapters under `src/portfolio/plan_adapters.rs`
