use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Error returned when a configuration name violates its public grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config name is invalid")]
pub struct ConfigNameError;

/// A bounded lowercase name in the durable configuration repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ConfigName(String);

impl ConfigName {
    /// Parses a configuration name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ConfigNameError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 64
            || !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit()
            || !bytes[bytes.len() - 1].is_ascii_lowercase()
                && !bytes[bytes.len() - 1].is_ascii_digit()
            || bytes
                .iter()
                .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'-')
        {
            return Err(ConfigNameError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the checked name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConfigName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConfigName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_have_one_checked_grammar() {
        for accepted in ["a", "0", "daily", "daily-main-2", &"a".repeat(64)] {
            assert_eq!(
                ConfigName::new(accepted).expect("accepted").as_str(),
                accepted
            );
        }
        for rejected in [
            "",
            "-daily",
            "daily-",
            "Daily",
            "daily_main",
            "daily/main",
            &"a".repeat(65),
        ] {
            assert!(ConfigName::new(rejected).is_err(), "accepted {rejected:?}");
        }
    }
}
