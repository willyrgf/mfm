# mfm-app

Application-facing orchestration bridge for MFM binaries and transports.

`mfm-app` keeps `bin/cli` and `bin/rest-api` transport-only by exposing:

- default engine, operation, and transport wiring
- environment-driven storage bootstrapping
- request/response helpers for starting, resuming, and inspecting runs
- higher-level built-in feature entrypoints such as portfolio snapshots

This crate depends on `mfm-sdk` and `mfm-machine`, but it does not own workflow logic. Planning
stays in op crates and executable behavior stays in shared-state crates.

## Example

```no_run
use mfm_app::{
    make_default_artifact_store, make_default_stream_store, make_engine_bundle, AppServices,
};

async fn boot() -> Result<AppServices, mfm_app::AppError> {
    let bundle = make_engine_bundle();
    let streams = make_default_stream_store().await?;
    let artifacts = make_default_artifact_store().await?;
    Ok(AppServices::new(bundle, streams, artifacts))
}
```
