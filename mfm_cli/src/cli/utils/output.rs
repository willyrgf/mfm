use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled};

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct KeyDisplay {
    pub id: String,
    pub label: String,
    pub key_type: String,
    #[tabled(skip)]
    pub address: Option<String>,
    pub created: String,
}

#[derive(Debug, Clone)]
pub enum OutputFormat {
    Table,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("Invalid output format: {s}")),
        }
    }
}

pub fn format_keys(keys: Vec<KeyDisplay>, format: OutputFormat, show_addresses: bool) -> String {
    match format {
        OutputFormat::Table => {
            if show_addresses {
                let mut table_data = Vec::new();
                for key in keys {
                    table_data.push(KeyDisplayWithAddress {
                        id: key.id,
                        label: key.label,
                        key_type: key.key_type,
                        address: key.address.unwrap_or_else(|| "N/A".to_string()),
                        created: key.created,
                    });
                }
                Table::new(table_data).to_string()
            } else {
                Table::new(keys).to_string()
            }
        }
        OutputFormat::Json => {
            if show_addresses {
                serde_json::to_string_pretty(&keys).unwrap_or_else(|_| "[]".to_string())
            } else {
                let keys_without_address: Vec<_> = keys
                    .into_iter()
                    .map(|mut k| {
                        k.address = None;
                        k
                    })
                    .collect();
                serde_json::to_string_pretty(&keys_without_address)
                    .unwrap_or_else(|_| "[]".to_string())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
struct KeyDisplayWithAddress {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub address: String,
    pub created: String,
}
