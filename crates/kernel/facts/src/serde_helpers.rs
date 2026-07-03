pub(crate) mod schema_id_serde {
    use mfm_ids::SchemaId;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S>(
        schema_id: &SchemaId,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(schema_id.as_str())
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> std::result::Result<SchemaId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        SchemaId::parse(&value).map_err(serde::de::Error::custom)
    }
}
