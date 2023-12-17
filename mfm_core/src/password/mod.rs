use crate::hidden::Hidden;
use serde_derive::{Deserialize, Serialize};
use std::{error::Error, fmt, str::FromStr};
use zeroize::Zeroize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SafePassword {
    password: Hidden<Box<[u8]>>,
}

impl From<String> for SafePassword {
    fn from(s: String) -> Self {
        Self {
            password: Hidden::from(s.into_bytes().into_boxed_slice()),
        }
    }
}

impl Drop for SafePassword {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

impl SafePassword {
    /// Gets a reference to bytes of a passphrase.
    pub fn reveal(&self) -> &[u8] {
        self.password.as_ref()
    }
}

/// An error for parsing a password from string.
#[derive(Debug)]
pub struct PasswordError;

impl Error for PasswordError {}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PasswordError")
    }
}

impl FromStr for SafePassword {
    type Err = PasswordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s.to_owned()))
    }
}

pub fn deserialize_safe_password<'de, D>(deserializer: D) -> Result<SafePassword, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    let password: String = Deserialize::deserialize(deserializer)?;
    Ok(SafePassword::from(password))
}
