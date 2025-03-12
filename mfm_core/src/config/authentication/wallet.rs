use serde::{Deserialize, Serialize};
use serde_json::Value;
use tari_utilities::SafePassword;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wallet {
    #[serde(deserialize_with = "deserialize_safe_password")]
    pub private_key: SafePassword,
    pub not_encrypted: Option<bool>,
    pub env_password: Option<String>,
}

impl PartialEq for Wallet {
    fn eq(&self, other: &Self) -> bool {
        self.not_encrypted == other.not_encrypted && self.env_password == other.env_password
    }
}

impl Eq for Wallet {}

fn deserialize_safe_password<'de, D>(deserializer: D) -> Result<SafePassword, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: Value = Deserialize::deserialize(deserializer)?;
    println!("Raw Value: {:?}", v);

    // let password: String = Deserialize::deserialize(deserializer)?;
    // println!("password: {}", password);
    // Ok(SafePassword::from(password))
    Ok(SafePassword::from(""))
}

#[cfg(test)]
mod test {
    use super::Wallet;
    //use super::deserialize_safe_password;
    //use serde::{Deserialize, Serialize};
    //use tari_utilities::SafePassword;

    // #[derive(Debug, Serialize, Deserialize)]
    // struct Wallet {
    //     #[serde(deserialize_with = "deserialize_safe_password")]
    //     private_key: SafePassword,
    // }

    #[test]
    fn test_wallet_deserializer() {
        let yaml_data = r#"
            private_key: "YOUR_PRIVATE_KEY"
        "#;
        let wallet: Wallet = serde_yaml::from_str(yaml_data).expect("Failed to deserialize");
        println!("wallet: {:?}", wallet.private_key.reveal());
    }
}
