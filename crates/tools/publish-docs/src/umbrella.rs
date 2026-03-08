use std::{collections::BTreeMap, fs, path::Path};

use crate::model::{
    CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, DocsRsObservation, DocsRsStatus,
    RegistryObservation, UmbrellaPolicy, Visibility,
};

const README_PATH: &str = "crates/docs/README.md";

/// Renders the umbrella README deterministically from catalog plus observed remote state.
pub fn render_readme(
    catalog: &DesiredCatalog,
    registry: &BTreeMap<String, RegistryObservation>,
    docs: &BTreeMap<String, DocsRsObservation>,
) -> String {
    let mut out = String::new();
    out.push_str("# mfm-docs\n\n");
    out.push_str("Umbrella documentation entry point for the MFM workspace.\n\n");
    out.push_str("`docs.rs` publishes crates one at a time and does not provide a workspace landing page. This crate fills that gap by acting as the top-level navigation page for the published MFM surface.\n\n");
    out.push_str("Only crates that are already live on docs.rs are linked below. Remaining workspace crates stay listed by path until their publish window completes.\n\n");
    out.push_str("Use this page to jump between crate families:\n\n");
    out.push_str("- engine and SDK\n");
    out.push_str("- core primitives\n");
    out.push_str("- shared states\n");
    out.push_str("- ops\n");
    out.push_str("- storages\n");
    out.push_str("- collectors\n");
    out.push_str("- transports\n");
    out.push_str("- binaries and tooling\n\n");
    out.push_str("The live runtime inventory still lives in the repository docs:\n\n");
    out.push_str("- `docs/ops-and-states.md`\n");
    out.push_str("- `docs/architecture.md`\n");
    out.push_str("- `docs/redesign.md`\n");

    for section in [
        CatalogSection::EngineSdk,
        CatalogSection::Core,
        CatalogSection::States,
        CatalogSection::Ops,
        CatalogSection::Storages,
        CatalogSection::Collectors,
        CatalogSection::Transports,
        CatalogSection::BinariesTooling,
    ] {
        let entries = section_entries(catalog, section);
        if entries.is_empty() {
            continue;
        }
        out.push_str("\n## ");
        out.push_str(section_title(section));
        out.push_str("\n\n");

        if section == CatalogSection::Ops {
            out.push_str(
                "Ops stay thin and focus on graph composition and config validation. Most are still repo-local today; publish them after the shared-state crates they depend on.\n\n",
            );
        }

        let show_docs = matches!(
            section,
            CatalogSection::EngineSdk
                | CatalogSection::Core
                | CatalogSection::States
                | CatalogSection::Collectors
        );
        if show_docs {
            out.push_str("| Package | Role | docs.rs | Workspace Path |\n");
            out.push_str("| --- | --- | --- | --- |\n");
            for package in entries {
                out.push_str(&format!(
                    "| `{}` | {} | {} | `{}` |\n",
                    package.name,
                    package.summary,
                    docs_cell(
                        package,
                        registry.get(&package.name),
                        docs.get(&package.name)
                    ),
                    package.workspace_path
                ));
            }
        } else {
            out.push_str("| Package | Role | Workspace Path |\n");
            out.push_str("| --- | --- | --- |\n");
            for package in entries {
                out.push_str(&format!(
                    "| `{}` | {} | `{}` |\n",
                    package.name, package.summary, package.workspace_path
                ));
            }
        }
    }

    out.push_str("\n## Publishing Notes\n\n");
    out.push_str("- Publish this crate after the first published docs.rs surface is live, then add links for later crates as they land.\n");
    out.push_str("- Keep this page role-oriented and high-level; detailed inventories belong in the repository docs.\n");
    out.push_str("- The publish order is tracked in `crates/docs/publish-wave.json` and consumed by `nix run .#publish-docs`.\n");
    out
}

/// Reads the current umbrella README from disk.
pub fn load_current_readme(workspace_root: &Path) -> Result<String, std::io::Error> {
    fs::read_to_string(workspace_root.join(README_PATH))
}

/// Writes the generated README if it changed. Returns whether the file changed.
pub fn sync_readme(workspace_root: &Path, generated: &str) -> Result<bool, std::io::Error> {
    let path = workspace_root.join(README_PATH);
    let current = fs::read_to_string(&path).unwrap_or_default();
    if current == generated {
        return Ok(false);
    }
    fs::write(path, generated)?;
    Ok(true)
}

fn section_entries(catalog: &DesiredCatalog, section: CatalogSection) -> Vec<&CatalogPackage> {
    let mut entries: Vec<_> = catalog
        .packages
        .iter()
        .filter(|package| {
            package.section == section
                && matches!(package.visibility, Visibility::Public)
                && !matches!(package.umbrella_policy, UmbrellaPolicy::Never)
        })
        .collect();
    entries.sort_by(|left, right| {
        right
            .release_priority
            .cmp(&left.release_priority)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

fn section_title(section: CatalogSection) -> &'static str {
    match section {
        CatalogSection::EngineSdk => "Engine And SDK",
        CatalogSection::Core => "Core Primitives",
        CatalogSection::States => "Shared States",
        CatalogSection::Ops => "Ops",
        CatalogSection::Storages => "Storages",
        CatalogSection::Collectors => "Collectors",
        CatalogSection::Transports => "Transports",
        CatalogSection::BinariesTooling => "Binaries And Tooling",
    }
}

fn docs_cell(
    package: &CatalogPackage,
    _registry: Option<&RegistryObservation>,
    docs: Option<&DocsRsObservation>,
) -> String {
    match package.docs_policy {
        DocsPolicy::RepoOnly | DocsPolicy::Hidden => "pending".to_string(),
        DocsPolicy::DocsRs => {
            if let Some(docs) = docs {
                match docs.status {
                    DocsRsStatus::Available => format!("<https://docs.rs/{}>", package.name),
                    DocsRsStatus::Failed => "failed".to_string(),
                    DocsRsStatus::Pending | DocsRsStatus::Absent => "pending".to_string(),
                    DocsRsStatus::TemporaryError => "unknown".to_string(),
                    DocsRsStatus::NotExpected => "pending".to_string(),
                }
            } else {
                "pending".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;

    use super::render_readme;
    use crate::model::{
        CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, DocsRsObservation,
        DocsRsStatus, RegistryObservation, RegistryStatus, UmbrellaPolicy, Visibility,
    };

    #[test]
    fn renders_pending_and_available_docs_rows() {
        let catalog = DesiredCatalog {
            catalog_version: 1,
            umbrella_package: "mfm-docs".into(),
            packages: vec![
                CatalogPackage {
                    name: "mfm-machine".into(),
                    workspace_path: "crates/machine".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::EngineSdk,
                    summary: "runtime".into(),
                    docs_policy: DocsPolicy::DocsRs,
                    umbrella_policy: UmbrellaPolicy::WhenPublished,
                    release_priority: 100,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                },
                CatalogPackage {
                    name: "mfm-sdk".into(),
                    workspace_path: "crates/sdk".into(),
                    visibility: Visibility::Public,
                    section: CatalogSection::EngineSdk,
                    summary: "sdk".into(),
                    docs_policy: DocsPolicy::DocsRs,
                    umbrella_policy: UmbrellaPolicy::WhenPublished,
                    release_priority: 90,
                    allow_yank: false,
                    owners: Vec::new(),
                    notes: String::new(),
                },
            ],
        };
        let mut registry = BTreeMap::new();
        registry.insert(
            "mfm-machine".into(),
            RegistryObservation {
                package: "mfm-machine".into(),
                status: RegistryStatus::Present,
                latest_version: Some(Version::parse("0.1.0").expect("version")),
                exact_version_present: true,
            },
        );
        registry.insert(
            "mfm-sdk".into(),
            RegistryObservation {
                package: "mfm-sdk".into(),
                status: RegistryStatus::Present,
                latest_version: Some(Version::parse("0.1.0").expect("version")),
                exact_version_present: true,
            },
        );
        let mut docs = BTreeMap::new();
        docs.insert(
            "mfm-machine".into(),
            DocsRsObservation {
                package: "mfm-machine".into(),
                status: DocsRsStatus::Available,
                latest_available_version: Some(Version::parse("0.1.0").expect("version")),
                exact_version_available: true,
            },
        );
        docs.insert(
            "mfm-sdk".into(),
            DocsRsObservation {
                package: "mfm-sdk".into(),
                status: DocsRsStatus::Pending,
                latest_available_version: None,
                exact_version_available: false,
            },
        );

        let rendered = render_readme(&catalog, &registry, &docs);
        assert!(rendered.contains("<https://docs.rs/mfm-machine>"));
        assert!(rendered.contains("| `mfm-sdk` | sdk | pending | `crates/sdk` |"));
    }
}
