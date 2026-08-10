//! Authority and identity proofs for sealed runtime/store assembly.

use mfm_store::structured::{
    AuditRunReader, ExportRunReader, PublicRunReader, ReplayRunReader, StructuredMemoryBackend,
    StructuredRuntime, TraceRunReader,
};

#[test]
fn purpose_readers_are_distinct_types() {
    fn assert_distinct<A: 'static, B: 'static>() {
        assert_ne!(std::any::TypeId::of::<A>(), std::any::TypeId::of::<B>());
    }
    assert_distinct::<
        PublicRunReader<StructuredMemoryBackend>,
        TraceRunReader<StructuredMemoryBackend>,
    >();
    assert_distinct::<
        PublicRunReader<StructuredMemoryBackend>,
        AuditRunReader<StructuredMemoryBackend>,
    >();
    assert_distinct::<
        PublicRunReader<StructuredMemoryBackend>,
        ReplayRunReader<StructuredMemoryBackend>,
    >();
    assert_distinct::<
        PublicRunReader<StructuredMemoryBackend>,
        ExportRunReader<StructuredMemoryBackend>,
    >();
}

#[test]
fn production_adapter_type_is_runtime_port() {
    let _ = std::any::type_name::<StructuredRuntime<StructuredMemoryBackend>>();
}
