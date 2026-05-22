use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct ManualValue {
    name: String,
}

impl mfm_values::MfmValue for ManualValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        let identity = mfm_values::SchemaIdentity::new(
            mfm_values::SchemaKind::Value,
            Some(Self::semantic_id()?),
            "mfm.trybuild.manual_value",
            mfm_ids::SchemaVersion::new("1").unwrap(),
            mfm_values::SchemaShape::named_struct(vec![mfm_values::FieldDescriptor::required(
                "name",
                mfm_values::SchemaShape::String,
            )])?,
        )?;
        mfm_values::SchemaDescriptor::new(
            identity,
            mfm_values::SchemaAudit::framework("domain_crate", "ManualValue"),
        )
    }

    fn semantic_id() -> mfm_values::Result<mfm_ids::SemanticTypeId> {
        mfm_ids::SemanticTypeId::new(
            "mfm.trybuild",
            "manual_value",
            "1",
            mfm_ids::DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0u8; 32]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

fn main() {}
