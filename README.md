# MFM

Experimental toolkit for recoverable on-chain operations built around a declaration-ordered
structured Runtime and append-only history.

> WARNING: Not production-ready. Do not use on mainnet.

## Architecture at a glance

```text
CLI / REST
  -> mfm-app (authorization and qualified assembly)
  -> authored State / Match / FanOut program
  -> pure expansion + certification
  -> Runtime (sole RunHistory writer and one-action interpreter)
       -> registered Read/Effect invokers -> transports, signers, resource authorities
  -> mfm-store (atomic append and sole callback-free fold)
       -> memory conformance or fenced PostgreSQL
  -> mfm-replay (reader-only projections and export)
```

The run journal has exactly five record families. Runtime durably authorizes every external
operation before possible entry and commits its observation before settlement. EVM nonce and
candidate uniqueness live in a separate narrow PostgreSQL wallet authority rather than the generic
kernel.

## Current product surface

The application publishes two entry points:

- `mfm.portfolio/snapshot@1`, backed by the structured portfolio and EVM balance programs; and
- `mfm.evm/submit-transaction@1`, backed by the registered structured submission expansion and
  qualified wallet authority.

Bitcoin model/transport support is retained, but no Bitcoin collection entry point is registered.

The repository's standalone CLI and REST bootstraps do not own deployment writer/session fencing
and therefore fail closed for authority-bearing application construction. Deployments embed the
libraries and inject a qualified application.

## Core properties

- declaration order, exhaustive Match, and bounded collect-all FanOut;
- one content-addressed certified program closure;
- exactly five append-only run record families and atomic object closure;
- one callback-free history fold shared by mutation, replay, trace, audit, and export;
- affine authorization/invocation/observation handling inside Runtime;
- append-only configured-value history with deployment write/app resolve roles;
- real PostgreSQL run-history and wallet-authority qualification;
- content-addressed, canonical, float-free persisted/public values; and
- no secrets, bearer bytes, provider text, or private authority in persisted surfaces.

## Documentation

- [Design contract](docs/design.md)
- [Architecture and responsibility placement](docs/architecture.md)
- [Structured run execution](docs/run-execution.md)
- [EVM transaction submission](docs/evm-transactions.md)
- [Portfolio snapshot](docs/portfolio-snapshot.md)
- [Persisted/public surfaces](docs/persisted-public-surfaces.md)
- [Build and verification](docs/build-and-verification.md)
- [Known gaps](docs/known-gaps.md)
- [Effect entry resolution](docs/effect-entry-resolution.md)
- [CLI](bin/cli/README.md) and [REST API](bin/rest-api/README.md)

## Development

All Cargo/Rust commands run through the pinned default Nix shell. Discover repository tasks and
binary help with:

```bash
nix run .#help
nix develop -c cargo run -p mfm -- --help
nix develop -c cargo run -p mfm-rest-api
```

Use the scope matrix in [docs/build-and-verification.md](docs/build-and-verification.md); do not
run broad gates merely because a commit is about to be created.

## License

MIT; see [LICENSE](LICENSE).
