# MFM - Redesign

## System Design requirements
- Append-only transactional
- Reproducibility: Nix philosophy applied at the architectural level of the whole system.
- Simplicity: KISS philosophy
- Composability & Reusability: less lines of code means less bugs and less security issue, we achieve it by having a better architecture.
- Auditability: everything must be trackable
- Every execution happens inside an state machine
- Its state machines all the way down (e.g., we can have a new state machine inside a state handler)


## Workspace Structure

```
mfm/
├── Cargo.toml          # workspace root, v0.1.29, resolver v2
├── crates/        
├──── collectors/         # EVM chains, Coingecko API, etc... 
├──── storages/           # ClickHouse, PostgreSQL, MinIO
├──── mfm_core/           # blockchain primitives, keystore, config, encryption primitives, wallets
├──── mfm_machine/        # generic async-ready state machine framework
├────── mfm_machine_derive/ # proc-macro for StateMetadata boilerplate (proc-macro lib, v0.1.0)
├──── ops/              # collection of all possible operations (collect->store->transform->reuse blockchain data)
├────── aave-tracker/   # track all AAVE big movements
├────── portfolio-tracker/   # track collection of wallets operations as portfolios
├────── portfolio-management/   # management a portfolio on-chain
├── bin/        
├──── mfm_cli/          # command-line interface
├──── rest_api/             # HTTP REST API
├── flake.nix           # nix build/dev environment
└── flake.lock
```
> Lets rename all these modules removing the prefix mfm_

### Dependency graph

```
{ mfm_cli, rest_api } -> ops -> { mfm_core -> mfm_machine -> mfm_machine_derive, collectors, storages }
```
