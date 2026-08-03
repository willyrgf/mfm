//! Authority and identity proofs for sealed runtime/store assembly.

use mfm_canonical::{CanonicalValue, RecoverabilityContract};
use mfm_ids::{InvocationIdentity, RunId, StableId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_runtime::history::StructuredStoreIdentity;
use mfm_store::structured::{
    AuditRunReader, ExportRunReader, PublicRunReader, ReplayRunReader, StoreHistoryAdapter,
    StructuredMemoryBackend, TraceRunReader,
};

fn store_identity() -> StructuredStoreIdentity {
    StructuredStoreIdentity {
        store_scope_id: StoreScopeId::new(format!(
            "{}{}",
            StoreScopeId::PREFIX,
            "1".repeat(32)
        ))
        .expect("scope"),
        store_epoch: StoreEpoch::new(1),
    }
}

fn derive_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    entry_point_operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> RunId {
    let preimage = CanonicalValue::object([
        (
            "store_scope_id",
            CanonicalValue::String(store_scope_id.as_str().to_owned()),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(tenant_scope_id.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            CanonicalValue::String(entry_point_operation_id.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(invocation_identity.as_str().to_owned()),
        ),
    ])
    .expect("preimage");
    let contract = RecoverabilityContract::embedded().expect("annex");
    let validated = contract
        .encode("mfm.run-id-preimage.v1", &preimage)
        .expect("encode");
    contract.derive_run_id(&validated).expect("run id")
}

#[test]
fn identical_run_id_preimages_derive_identical_ids() {
    let identity = store_identity();
    let tenant = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
        .expect("tenant");
    let operation = StableId::new("mfm.test/entry").expect("op");
    let invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000001").expect("inv");
    let first = derive_run_id(
        &identity.store_scope_id,
        &tenant,
        &operation,
        &invocation,
    );
    let second = derive_run_id(
        &identity.store_scope_id,
        &tenant,
        &operation,
        &invocation,
    );
    assert_eq!(first, second);
}

#[test]
fn each_run_id_preimage_field_changes_identity() {
    let identity = store_identity();
    let tenant = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
        .expect("tenant");
    let operation = StableId::new("mfm.test/entry").expect("op");
    let invocation =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000001").expect("inv");
    let base = derive_run_id(
        &identity.store_scope_id,
        &tenant,
        &operation,
        &invocation,
    );
    let other_scope = StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "3".repeat(32)))
        .expect("scope");
    assert_ne!(
        base,
        derive_run_id(&other_scope, &tenant, &operation, &invocation)
    );
    let other_tenant = TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "4".repeat(32)))
        .expect("tenant");
    assert_ne!(
        base,
        derive_run_id(
            &identity.store_scope_id,
            &other_tenant,
            &operation,
            &invocation
        )
    );
    let other_op = StableId::new("mfm.test/other-entry").expect("op");
    assert_ne!(
        base,
        derive_run_id(
            &identity.store_scope_id,
            &tenant,
            &other_op,
            &invocation
        )
    );
    let other_inv =
        InvocationIdentity::new("00000000-0000-4000-8000-000000000099").expect("inv");
    assert_ne!(
        base,
        derive_run_id(
            &identity.store_scope_id,
            &tenant,
            &operation,
            &other_inv
        )
    );
}

#[test]
fn purpose_readers_are_distinct_types() {
    fn assert_distinct<A: 'static, B: 'static>() {
        assert_ne!(std::any::TypeId::of::<A>(), std::any::TypeId::of::<B>());
    }
    assert_distinct::<PublicRunReader<StructuredMemoryBackend>, TraceRunReader<StructuredMemoryBackend>>();
    assert_distinct::<PublicRunReader<StructuredMemoryBackend>, AuditRunReader<StructuredMemoryBackend>>();
    assert_distinct::<PublicRunReader<StructuredMemoryBackend>, ReplayRunReader<StructuredMemoryBackend>>();
    assert_distinct::<PublicRunReader<StructuredMemoryBackend>, ExportRunReader<StructuredMemoryBackend>>();
}

#[test]
fn production_adapter_type_is_runtime_port() {
    let _ = std::any::type_name::<
        mfm_runtime::structured::Runtime<StoreHistoryAdapter<StructuredMemoryBackend>>,
    >();
}
