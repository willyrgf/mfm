use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallet {
    import_private_key: String,
}
#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct General {
    wallet: Wallet,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    general: General,
}

impl Config {
    pub fn from_file(f: &str) -> Self {
        let reader = std::fs::File::open(f).unwrap();
        let config: Config = serde_yaml::from_reader(reader).unwrap();
        config
    }
}
