use crate::utils::hidden::Hidden;
use magic_crypt::{new_magic_crypt, MagicCryptTrait};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, str::FromStr};
use zeroize::Zeroize;
use zeroize::Zeroizing;

pub const MFM_WALLET_PASSWORD: &str = "MFM_WALLET_PASSWORD";

/// A hidden string that implements [`Zeroize`].
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

/// Gets the password provided by command line argument or environment variable if available.
/// Otherwise prompts for the password to be typed in.
pub fn get_or_prompt_password(env_password: Option<String>) -> Result<SafePassword, anyhow::Error> {
    let env_value = match env_password {
        Some(env_password) => std::env::var_os(env_password),
        None => std::env::var_os(MFM_WALLET_PASSWORD),
    };

    if let Some(p) = env_value {
        let env_password = p.into_string().map_err(|e| {
            anyhow::anyhow!("failed to convert OsString into String, error: {:?}", e)
        })?;
        return Ok(env_password.into());
    }

    let password = prompt_password("Wallet password: ")?;

    Ok(password)
}

pub fn prompt_password(prompt: &str) -> Result<SafePassword, anyhow::Error> {
    let password = loop {
        let pass = rpassword::prompt_password(prompt)
            .map_err(|e| anyhow::anyhow!("failed to convert OsString into String, error: {}", e))?;
        if pass.is_empty() {
            println!("Password cannot be empty!");
            continue;
        } else {
            break pass;
        }
    };

    Ok(SafePassword::from(password))
}

pub fn decrypt_private_key(
    password: SafePassword,
    private_key: SafePassword,
) -> Result<SafePassword, anyhow::Error> {
    let str_password = {
        let bytes = password.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    };

    let str_private_key = {
        let bytes = private_key.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    };

    let mc = new_magic_crypt!(str_password, 256);

    match mc.decrypt_base64_to_string(str_private_key) {
        Ok(s) => Ok(SafePassword::from(s)),
        _ => Err(anyhow::anyhow!("invalid password")),
    }
}

pub fn encrypt_private_key_to_base64(password: SafePassword, private_key: SafePassword) -> String {
    let str_password = {
        let bytes = password.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    };

    let mc = new_magic_crypt!(str_password, 256);

    mc.encrypt_bytes_to_base64(private_key.reveal())
}

mod test {
    #[test]
    fn test_encrypt_decrypt() {
        use super::{decrypt_private_key, encrypt_private_key_to_base64, SafePassword};

        let password = SafePassword::from("some pass".to_string());
        let private_key =
            SafePassword::from("10ED43C718714eb63d5aA57B78B54704E256024E".to_string());

        let encrypted_base64 = encrypt_private_key_to_base64(password.clone(), private_key.clone());

        let decrypted_private_key =
            decrypt_private_key(password, SafePassword::from(encrypted_base64)).unwrap();

        assert_eq!(private_key, decrypted_private_key)
    }
}
