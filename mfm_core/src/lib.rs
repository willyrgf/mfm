use std::{fs::File, io::Read};

pub mod config;
pub mod contexts;
pub mod hidden;
pub mod operations;
pub mod password;
pub mod states;

use anyhow::Error;
use serde::de::DeserializeOwned;

fn read_yaml<T: DeserializeOwned>(path: String) -> Result<T, Error> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let instance: T = serde_yaml::from_str(&contents)?;
    Ok(instance)
}
