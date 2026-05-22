use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct ManualOutput {
    name: String,
}

impl mfm_values::OperationOutput for ManualOutput {
    fn output_schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        let identity = mfm_values::SchemaIdentity::new(
            mfm_values::SchemaKind::OperationOutput,
            None,
            "mfm.trybuild.manual_operation_output",
            mfm_ids::SchemaVersion::new("1").unwrap(),
            mfm_values::SchemaShape::named_struct(vec![mfm_values::FieldDescriptor::required(
                "name",
                mfm_values::SchemaShape::String,
            )])?,
        )?;
        mfm_values::SchemaDescriptor::new(
            identity,
            mfm_values::SchemaAudit::framework("domain_crate", "ManualOutput"),
        )
    }
}

fn main() {}
