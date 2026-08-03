// Production writers are not public. This fails at the import boundary.
use mfm_store::structured::StructuredRunHistoryWriter;

fn main() {
    let _ = std::any::type_name::<StructuredRunHistoryWriter<()>>();
}
