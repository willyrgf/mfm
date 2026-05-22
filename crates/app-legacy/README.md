# mfm-app-legacy

Legacy dynamic application-facing orchestration bridge for MFM binaries and transports.

`mfm-app-legacy` keeps the pre-typed `bin/cli` and `bin/rest-api` surfaces compiling while those
binaries are rewritten. It is not a certified typed submit/resume surface and must not be used by
new typed runtime entrypoints.

It exposes the old dynamic bridge:

- default engine, operation, and transport wiring
- environment-driven storage bootstrapping
- request/response helpers for starting, resuming, and inspecting runs
- higher-level built-in feature entrypoints such as portfolio snapshots

This crate depends on `mfm-sdk` and `mfm-machine`. It is isolated from the typed `mfm-app` crate so
old dynamic planning cannot influence certified typed runs.

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
