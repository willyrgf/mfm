use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct ManualPublicOutput {
    name: String,
}

impl mfm_values::PublicOutputDescriptor for ManualPublicOutput {
    fn public_schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        let identity = mfm_values::SchemaIdentity::new(
            mfm_values::SchemaKind::PublicOutput,
            None,
            "mfm.trybuild.manual_public_output",
            mfm_ids::SchemaVersion::new("1").unwrap(),
            mfm_values::SchemaShape::named_struct(vec![mfm_values::FieldDescriptor::required(
                "name",
                mfm_values::SchemaShape::String,
            )])?,
        )?;
        mfm_values::SchemaDescriptor::new(
            identity,
            mfm_values::SchemaAudit::framework("domain_crate", "ManualPublicOutput"),
        )
    }
}

impl mfm_values::PublicOutputs for ManualPublicOutput {}

fn main() {}
