# mfm-machine-test-support

Shared contract tests and observability helpers for `mfm-machine` backends.

Use this crate when implementing a new `EventStore` or `ArtifactStore` and you want the same
conformance checks that the built-in storage crates run in CI.

## Example

```rust
use mfm_machine::stores::ArtifactStore;
use mfm_machine_test_support::{artifact_store_contract_tests, init_test_observability};

async fn assert_backend(store: &dyn ArtifactStore) {
    init_test_observability();
    artifact_store_contract_tests(store).await;
}
```
