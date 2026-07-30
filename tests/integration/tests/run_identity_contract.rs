use mfm_canonical::{CanonicalValue, RecoverabilityContract};
use mfm_ids::{InvocationIdentity, StableId, StoreScopeId, TenantScopeId};

#[test]
fn run_identity_is_deterministic_and_tenant_scoped() {
    let store_scope_id = StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "1".repeat(32)))
        .expect("store scope");
    let operation = StableId::new("mfm.portfolio/snapshot").expect("operation");
    let invocation =
        InvocationIdentity::new("de305d54-75b4-431b-adb2-eb6b9e546014").expect("invocation");
    let tenant_a = tenant('a');
    let tenant_b = tenant('b');

    let first = derive_run_id(&store_scope_id, &tenant_a, &operation, &invocation);
    assert_eq!(
        derive_run_id(&store_scope_id, &tenant_a, &operation, &invocation),
        first,
        "an exact logical admission must retain one deterministic run id"
    );
    assert_ne!(
        derive_run_id(&store_scope_id, &tenant_b, &operation, &invocation),
        first,
        "the policy-derived tenant must participate in run identity"
    );
}

fn derive_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    operation: &StableId,
    invocation: &InvocationIdentity,
) -> mfm_ids::RunId {
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
            CanonicalValue::String(operation.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(invocation.as_str().to_owned()),
        ),
    ])
    .expect("run-id preimage");
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    let validated = contract
        .encode("mfm.run-id-preimage.v1", &preimage)
        .expect("validated run-id preimage");
    contract
        .derive_run_id(&validated)
        .expect("derive run identity")
}

fn tenant(marker: char) -> TenantScopeId {
    TenantScopeId::new(format!(
        "{}{}",
        TenantScopeId::PREFIX,
        marker.to_string().repeat(32)
    ))
    .expect("tenant scope")
}
