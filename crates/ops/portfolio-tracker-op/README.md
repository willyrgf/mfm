# mfm-op-portfolio-tracker

Portfolio tracking operation (`op_id = "portfolio_tracker"`, `op_version = "v1"`).

This op is the thin planner for the canonical multi-network portfolio snapshot flow. It validates
the canonical `portfolio` plus `valuation_source_registry` config surfaces and wires the reusable
wallet, symbol, Aave, and portfolio shared states.

Docs:
- `REVAMP_PORTFOLIO_SNAPSHOT.md`
- `docs/redesign.md`
