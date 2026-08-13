use mfm_ids::{StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_store::QualifiedRun;

fn main() {
    let _ = QualifiedRun::qualify_prefix(
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").unwrap(),
        StoreEpoch::new(1),
        TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef").unwrap(),
        Vec::new(),
    );
}
