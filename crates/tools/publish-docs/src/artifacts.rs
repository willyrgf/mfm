use std::{fs, path::Path};

use serde::Serialize;

use crate::model::{
    ApplyResult, DesiredCatalog, DocsRsObservation, Mode, Plan, PublishWave, RegistryObservation,
    SummaryCounts, WorkspaceState,
};

/// Summary artifact written alongside plan and result payloads.
#[derive(Debug, Serialize)]
pub(crate) struct SummaryArtifact<'a> {
    /// Schema version for machine-readable consumers.
    pub schema_version: u32,
    /// Stable run identifier.
    pub run_id: &'a str,
    /// Selected mode string.
    pub mode: &'a str,
    /// Wave name.
    pub wave: &'a str,
    /// Aggregated counts by action.
    pub summary: &'a SummaryCounts,
}

/// Writes sanitized artifacts for the current run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_run_artifacts(
    artifact_dir: &Path,
    wave: &PublishWave,
    catalog: &DesiredCatalog,
    workspace: &WorkspaceState,
    registry: &[RegistryObservation],
    docs: &[DocsRsObservation],
    plan: &Plan,
    summary: &SummaryCounts,
    generated_readme: &str,
    apply_result: Option<&ApplyResult>,
) -> Result<(), std::io::Error> {
    fs::create_dir_all(artifact_dir)?;
    write_json(artifact_dir.join("wave.json"), wave)?;
    write_json(artifact_dir.join("catalog.json"), catalog)?;
    write_json(artifact_dir.join("local.json"), workspace)?;
    write_json(artifact_dir.join("registry.json"), registry)?;
    write_json(artifact_dir.join("docs.json"), docs)?;
    write_json(artifact_dir.join("plan.json"), plan)?;
    write_json(
        artifact_dir.join("summary.json"),
        &SummaryArtifact {
            schema_version: 1,
            run_id: &plan.run_id,
            mode: mode_label(plan.mode),
            wave: &plan.wave,
            summary,
        },
    )?;
    fs::write(artifact_dir.join("umbrella-preview.md"), generated_readme)?;
    if let Some(result) = apply_result {
        write_json(artifact_dir.join("results.json"), result)?;
    }
    Ok(())
}

fn write_json(
    path: impl AsRef<Path>,
    value: &(impl Serialize + ?Sized),
) -> Result<(), std::io::Error> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    fs::write(path, bytes)
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Plan => "plan",
        Mode::Apply => "apply",
        Mode::Resume => "resume",
        Mode::SyncUmbrella => "sync_umbrella",
        Mode::Yank => "yank",
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;
    use tempfile::tempdir;

    use super::write_run_artifacts;
    use crate::model::{
        CatalogPackage, CatalogSection, DesiredCatalog, DocsPolicy, DocsRsStatus, Mode, Plan,
        PlanAction, PlannedPackage, PublishWave, RegistryFreshness, RegistryObservation,
        RegistryObservationSource, RegistryStatus, SummaryCounts, UmbrellaPolicy, Visibility,
        WavePackage, WorkspaceState,
    };

    #[test]
    fn writes_expected_artifact_files() {
        let dir = tempdir().expect("tempdir");
        let wave = PublishWave {
            wave: "docs-rs-wave-1".into(),
            packages: vec![WavePackage {
                name: "mfm-runtime".into(),
                group: "foundation".into(),
                workspace_path: "crates/kernel/runtime".into(),
                docs_rs: "https://docs.rs/mfm-runtime".into(),
            }],
        };
        let catalog = DesiredCatalog {
            catalog_version: 1,
            umbrella_package: "mfm-docs".into(),
            packages: vec![CatalogPackage {
                name: "mfm-runtime".into(),
                workspace_path: "crates/kernel/runtime".into(),
                visibility: Visibility::Public,
                section: CatalogSection::EngineSdk,
                summary: "runtime".into(),
                docs_policy: DocsPolicy::DocsRs,
                umbrella_policy: UmbrellaPolicy::WhenPublished,
                release_priority: 100,
                allow_yank: false,
                owners: Vec::new(),
                notes: String::new(),
            }],
        };
        let workspace = WorkspaceState {
            selected: vec!["mfm-runtime".into()],
            packages: vec![],
        };
        let registry = vec![RegistryObservation {
            package: "mfm-runtime".into(),
            status: RegistryStatus::Absent,
            latest_version: None,
            exact_version_present: false,
            source: RegistryObservationSource::Index,
            freshness: RegistryFreshness::Fresh,
            observed_at: Some("2026-03-09T00:00:00Z".into()),
            diagnostic_code: None,
        }];
        let docs = vec![crate::model::DocsRsObservation {
            package: "mfm-runtime".into(),
            status: DocsRsStatus::Absent,
            latest_available_version: None,
            exact_version_available: false,
        }];
        let plan = Plan {
            schema_version: 1,
            run_id: "run_1".into(),
            mode: Mode::Plan,
            wave: "docs-rs-wave-1".into(),
            selection_from: None,
            selection_only: None,
            resumed_from_run_id: None,
            packages: vec![PlannedPackage {
                name: "mfm-runtime".into(),
                local_version: Version::parse("0.1.0").expect("version"),
                remote_version: None,
                docs_status: Some(DocsRsStatus::Absent),
                action: PlanAction::Publish,
                reason: "target-version-absent".into(),
                blocking_dependencies: Vec::new(),
                synthetic: false,
            }],
        };
        let summary = SummaryCounts {
            publish: 1,
            ..SummaryCounts::default()
        };

        write_run_artifacts(
            dir.path(),
            &wave,
            &catalog,
            &workspace,
            &registry,
            &docs,
            &plan,
            &summary,
            "# mfm-docs\n",
            None,
        )
        .expect("write artifacts");

        for file in [
            "wave.json",
            "catalog.json",
            "local.json",
            "registry.json",
            "docs.json",
            "plan.json",
            "summary.json",
            "umbrella-preview.md",
        ] {
            assert!(dir.path().join(file).is_file(), "{file} missing");
        }
        assert!(!dir.path().join("results.json").exists());
    }
}
