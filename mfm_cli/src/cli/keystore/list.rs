use crate::cli::utils::{
    keystore::KeystoreManager,
    output::{format_keys, KeyDisplay, OutputFormat},
};
use clap::Args;
use regex::Regex;
use std::path::PathBuf;

#[derive(Args)]
pub struct ListArgs {
    /// Keystore file path
    #[arg(long)]
    pub keystore: Option<PathBuf>,

    /// Output format: table, json
    #[arg(long, default_value = "table")]
    pub format: OutputFormat,

    /// Include Ethereum addresses in output
    #[arg(long)]
    pub show_addresses: bool,

    /// Filter by label pattern (regex supported)
    #[arg(long)]
    pub filter_label: Option<String>,

    /// Sort by: label, created, type
    #[arg(long, default_value = "created")]
    pub sort_by: SortBy,
}

#[derive(clap::ValueEnum, Clone)]
pub enum SortBy {
    #[value(name = "label")]
    Label,
    #[value(name = "created")]
    Created,
    #[value(name = "type")]
    Type,
}

pub async fn execute(args: &ListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let manager = KeystoreManager::new(args.keystore.clone());
    let keystore = manager.get_unlocked_keystore().await?;

    let keys = keystore.list_keys()?;

    if keys.is_empty() {
        println!("No keys found in keystore");
        return Ok(());
    }

    let mut key_displays: Vec<KeyDisplay> = keys
        .into_iter()
        .map(|key| KeyDisplay {
            id: key.id.to_string(),
            label: key
                .alias
                .clone()
                .unwrap_or_else(|| "<no alias>".to_string()),
            key_type: format!("{:?}", key.key_type).to_lowercase(),
            address: if args.show_addresses {
                Some(format!("{:?}", key.address))
            } else {
                None
            },
            created: key.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    // Apply label filter if specified
    if let Some(filter_pattern) = &args.filter_label {
        let regex = Regex::new(filter_pattern)?;
        key_displays.retain(|key| regex.is_match(&key.label));
    }

    // Sort keys
    match args.sort_by {
        SortBy::Label => key_displays.sort_by(|a, b| a.label.cmp(&b.label)),
        SortBy::Created => key_displays.sort_by(|a, b| a.created.cmp(&b.created)),
        SortBy::Type => key_displays.sort_by(|a, b| a.key_type.cmp(&b.key_type)),
    }

    let output = format_keys(key_displays, args.format.clone(), args.show_addresses);
    println!("{output}");

    Ok(())
}
