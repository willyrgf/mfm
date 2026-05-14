//! Internal authored and canonical config pipeline for `publish-docs`.
//!
//! Transport boundaries load authored TOML or JSON into typed authored config and immediately
//! canonicalize it before workspace-aware validation or plan building happens.
//!
//! This module owns only format parsing and default materialization. Semantic validation against the
//! local workspace remains with the `mfm-publish-docs` workflow family.

use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Desired public visibility for a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Crate is part of the intended public surface.
    Public,
    /// Crate is internal and should not be treated as public surface.
    Private,
    /// Crate is retired and may require operator review or yanking.
    Retired,
}

/// Docs hosting policy for a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocsPolicy {
    /// `docs.rs` is the intended docs surface.
    #[serde(rename = "docs-rs")]
    DocsRs,
    /// The crate should appear only as a repo-local surface.
    #[serde(rename = "repo-only")]
    RepoOnly,
    /// The crate should be hidden from docs surfacing.
    #[serde(rename = "hidden")]
    Hidden,
}

/// Umbrella inclusion policy for a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UmbrellaPolicy {
    /// Always show on the umbrella page.
    #[serde(rename = "always")]
    Always,
    /// Show only once the crate is published or otherwise externally visible.
    #[serde(rename = "when-published")]
    WhenPublished,
    /// Never show on the umbrella page.
    #[serde(rename = "never")]
    Never,
}

/// Ordering section for umbrella generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CatalogSection {
    /// Engine and SDK crates.
    #[serde(rename = "engine_sdk")]
    EngineSdk,
    /// Core primitives.
    #[serde(rename = "core")]
    Core,
    /// Shared states.
    #[serde(rename = "states")]
    States,
    /// Ops.
    #[serde(rename = "ops")]
    Ops,
    /// Storage crates.
    #[serde(rename = "storages")]
    Storages,
    /// Collector crates.
    #[serde(rename = "collectors")]
    Collectors,
    /// Transport crates.
    #[serde(rename = "transports")]
    Transports,
    /// Binaries and tooling.
    #[serde(rename = "binaries_tooling")]
    BinariesTooling,
}

/// Human-authored publish wave entry loaded from TOML or JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WavePackageAuthoredConfig {
    /// Cargo package name.
    pub name: String,
    /// Human-facing wave group name.
    pub group: String,
    /// Relative workspace path.
    pub workspace_path: String,
    /// Expected docs.rs URL recorded in the wave file.
    pub docs_rs: String,
}

/// Human-authored publish wave loaded from TOML or JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishWaveAuthoredConfig {
    /// Wave name.
    pub wave: String,
    /// Ordered packages in the wave.
    pub packages: Vec<WavePackageAuthoredConfig>,
}

/// Human-authored desired catalog entry loaded from TOML or JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPackageAuthoredConfig {
    /// Cargo package name.
    pub name: String,
    /// Relative workspace path.
    pub workspace_path: String,
    /// Desired visibility.
    pub visibility: Visibility,
    /// Umbrella section.
    pub section: CatalogSection,
    /// Human summary used for README generation.
    pub summary: String,
    /// Docs policy for the crate.
    #[serde(default)]
    pub docs_policy: Option<DocsPolicy>,
    /// Umbrella policy for the crate.
    #[serde(default)]
    pub umbrella_policy: Option<UmbrellaPolicy>,
    /// Relative ordering inside a section.
    #[serde(default)]
    pub release_priority: Option<i64>,
    /// Whether yanking is allowed by policy.
    #[serde(default)]
    pub allow_yank: Option<bool>,
    /// Optional owner hints.
    #[serde(default)]
    pub owners: Option<Vec<String>>,
    /// Freeform notes.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Human-authored desired-state catalog loaded from TOML or JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredCatalogAuthoredConfig {
    /// Schema version.
    pub catalog_version: u32,
    /// Package name of the umbrella crate.
    pub umbrella_package: String,
    /// Catalog entries.
    pub packages: Vec<CatalogPackageAuthoredConfig>,
}

/// Canonical desired catalog entry with defaults materialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPackage {
    /// Cargo package name.
    pub name: String,
    /// Relative workspace path.
    pub workspace_path: String,
    /// Desired visibility.
    pub visibility: Visibility,
    /// Umbrella section.
    pub section: CatalogSection,
    /// Human summary used for README generation.
    pub summary: String,
    /// Docs policy for the crate.
    pub docs_policy: DocsPolicy,
    /// Umbrella policy for the crate.
    pub umbrella_policy: UmbrellaPolicy,
    /// Relative ordering inside a section.
    pub release_priority: i64,
    /// Whether yanking is allowed by policy.
    pub allow_yank: bool,
    /// Optional owner hints.
    pub owners: Vec<String>,
    /// Freeform notes.
    pub notes: String,
}

/// Canonical desired-state catalog used by `publish-docs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredCatalog {
    /// Schema version.
    pub catalog_version: u32,
    /// Package name of the umbrella crate.
    pub umbrella_package: String,
    /// Catalog entries.
    pub packages: Vec<CatalogPackage>,
}

/// Canonical publish wave entry used by `publish-docs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WavePackage {
    /// Cargo package name.
    pub name: String,
    /// Human-facing wave group name.
    pub group: String,
    /// Relative workspace path.
    pub workspace_path: String,
    /// Expected docs.rs URL recorded in the wave file.
    pub docs_rs: String,
}

/// Canonical publish wave used by `publish-docs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishWave {
    /// Wave name from the authored config.
    pub wave: String,
    /// Ordered packages in the wave.
    pub packages: Vec<WavePackage>,
}

/// Errors returned while parsing or canonicalizing desired catalog config.
#[derive(Debug, Error)]
pub enum PublishDocsCatalogConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse publish-docs desired catalog as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse publish-docs desired catalog as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
    },
}

/// Errors returned while parsing or canonicalizing publish wave config.
#[derive(Debug, Error)]
pub enum PublishDocsWaveConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse publish-docs wave config as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse publish-docs wave config as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
    },
}

/// Parses authored publish wave config using the supplied format.
pub fn parse_publish_wave_authored_config(
    raw: &str,
    format: AuthoredConfigFormat,
) -> Result<PublishWaveAuthoredConfig, PublishDocsWaveConfigError> {
    match format {
        AuthoredConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|source| PublishDocsWaveConfigError::InvalidJson { source }),
        AuthoredConfigFormat::Toml => {
            toml::from_str(raw).map_err(|source| PublishDocsWaveConfigError::InvalidToml { source })
        }
    }
}

/// Parses authored publish wave config using an optional path hint.
pub fn parse_publish_wave_authored_config_with_hint(
    raw: &str,
    path_hint: Option<&Path>,
) -> Result<PublishWaveAuthoredConfig, PublishDocsWaveConfigError> {
    parse_authored_config_with_hint(
        raw,
        path_hint,
        |value| parse_publish_wave_authored_config(value, AuthoredConfigFormat::Json),
        |value| parse_publish_wave_authored_config(value, AuthoredConfigFormat::Toml),
    )
}

/// Canonicalizes authored publish wave config.
pub fn canonicalize_publish_wave_authored_config(
    authored: PublishWaveAuthoredConfig,
) -> Result<PublishWave, PublishDocsWaveConfigError> {
    Ok(PublishWave {
        wave: authored.wave,
        packages: authored
            .packages
            .into_iter()
            .map(|package| WavePackage {
                name: package.name,
                group: package.group,
                workspace_path: package.workspace_path,
                docs_rs: package.docs_rs,
            })
            .collect(),
    })
}

/// Parses authored desired catalog config using the supplied format.
pub fn parse_desired_catalog_authored_config(
    raw: &str,
    format: AuthoredConfigFormat,
) -> Result<DesiredCatalogAuthoredConfig, PublishDocsCatalogConfigError> {
    match format {
        AuthoredConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|source| PublishDocsCatalogConfigError::InvalidJson { source }),
        AuthoredConfigFormat::Toml => toml::from_str(raw)
            .map_err(|source| PublishDocsCatalogConfigError::InvalidToml { source }),
    }
}

/// Parses authored desired catalog config using an optional path hint.
pub fn parse_desired_catalog_authored_config_with_hint(
    raw: &str,
    path_hint: Option<&Path>,
) -> Result<DesiredCatalogAuthoredConfig, PublishDocsCatalogConfigError> {
    parse_authored_config_with_hint(
        raw,
        path_hint,
        |value| parse_desired_catalog_authored_config(value, AuthoredConfigFormat::Json),
        |value| parse_desired_catalog_authored_config(value, AuthoredConfigFormat::Toml),
    )
}

/// Canonicalizes authored desired catalog config by materializing defaults.
pub fn canonicalize_desired_catalog_authored_config(
    authored: DesiredCatalogAuthoredConfig,
) -> Result<DesiredCatalog, PublishDocsCatalogConfigError> {
    Ok(DesiredCatalog {
        catalog_version: authored.catalog_version,
        umbrella_package: authored.umbrella_package,
        packages: authored
            .packages
            .into_iter()
            .map(|package| CatalogPackage {
                name: package.name,
                workspace_path: package.workspace_path,
                visibility: package.visibility,
                section: package.section,
                summary: package.summary,
                docs_policy: package.docs_policy.unwrap_or(DocsPolicy::RepoOnly),
                umbrella_policy: package
                    .umbrella_policy
                    .unwrap_or(UmbrellaPolicy::WhenPublished),
                release_priority: package.release_priority.unwrap_or(100),
                allow_yank: package.allow_yank.unwrap_or(false),
                owners: package.owners.unwrap_or_default(),
                notes: package.notes.unwrap_or_default(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_WAVE: &str = r#"{
        "wave": "docs-rs-wave-1",
        "packages": [{
            "name": "mfm-docs",
            "group": "umbrella",
            "workspace_path": "crates/docs",
            "docs_rs": "https://docs.rs/mfm-docs"
        }]
    }"#;

    const TOML_WAVE: &str = r#"
        wave = "docs-rs-wave-1"

        [[packages]]
        name = "mfm-docs"
        group = "umbrella"
        workspace_path = "crates/docs"
        docs_rs = "https://docs.rs/mfm-docs"
    "#;

    const JSON_CATALOG: &str = r#"{
        "catalog_version": 1,
        "umbrella_package": "mfm-docs",
        "packages": [{
            "name": "mfm-docs",
            "workspace_path": "crates/docs",
            "visibility": "public",
            "section": "binaries_tooling",
            "summary": "Umbrella docs surface"
        }]
    }"#;

    const TOML_CATALOG: &str = r#"
        catalog_version = 1
        umbrella_package = "mfm-docs"

        [[packages]]
        name = "mfm-docs"
        workspace_path = "crates/docs"
        visibility = "public"
        section = "binaries_tooling"
        summary = "Umbrella docs surface"
    "#;

    #[test]
    fn json_and_toml_authoring_normalize_to_same_canonical_catalog() {
        let json = canonicalize_desired_catalog_authored_config(
            parse_desired_catalog_authored_config(JSON_CATALOG, AuthoredConfigFormat::Json)
                .expect("json parse"),
        )
        .expect("json canonical");
        let toml = canonicalize_desired_catalog_authored_config(
            parse_desired_catalog_authored_config_with_hint(
                TOML_CATALOG,
                Some(Path::new("catalog.toml")),
            )
            .expect("toml parse"),
        )
        .expect("toml canonical");

        assert_eq!(json, toml);
    }

    #[test]
    fn canonicalization_materializes_catalog_defaults() {
        let canonical = canonicalize_desired_catalog_authored_config(
            parse_desired_catalog_authored_config(JSON_CATALOG, AuthoredConfigFormat::Json)
                .expect("json parse"),
        )
        .expect("canonical");

        let package = &canonical.packages[0];
        assert_eq!(package.docs_policy, DocsPolicy::RepoOnly);
        assert_eq!(package.umbrella_policy, UmbrellaPolicy::WhenPublished);
        assert_eq!(package.release_priority, 100);
        assert!(!package.allow_yank);
        assert!(package.owners.is_empty());
        assert!(package.notes.is_empty());
    }

    #[test]
    fn hintless_parse_falls_back_from_json_to_toml() {
        let authored = parse_desired_catalog_authored_config_with_hint(TOML_CATALOG, None)
            .expect("toml parse without hint");

        assert_eq!(authored.catalog_version, 1);
        assert_eq!(authored.umbrella_package, "mfm-docs");
    }

    #[test]
    fn publish_wave_json_and_toml_authoring_normalize_to_same_canonical_wave() {
        let json = canonicalize_publish_wave_authored_config(
            parse_publish_wave_authored_config(JSON_WAVE, AuthoredConfigFormat::Json)
                .expect("json parse"),
        )
        .expect("json canonical");
        let toml = canonicalize_publish_wave_authored_config(
            parse_publish_wave_authored_config_with_hint(TOML_WAVE, Some(Path::new("wave.toml")))
                .expect("toml parse"),
        )
        .expect("toml canonical");

        assert_eq!(json, toml);
    }

    #[test]
    fn publish_wave_hintless_parse_falls_back_from_json_to_toml() {
        let authored = parse_publish_wave_authored_config_with_hint(TOML_WAVE, None)
            .expect("toml parse without hint");

        assert_eq!(authored.wave, "docs-rs-wave-1");
        assert_eq!(authored.packages.len(), 1);
        assert_eq!(authored.packages[0].name, "mfm-docs");
    }
}
