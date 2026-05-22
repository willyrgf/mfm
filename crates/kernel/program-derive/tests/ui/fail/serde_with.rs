use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

mod string_codec {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(value)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)
    }
}

#[derive(Clone, Serialize, Deserialize, MfmValue)]
struct BadValue {
    #[serde(with = "string_codec")]
    name: String,
}

fn main() {}
