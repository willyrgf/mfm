use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

/// Output formats supported by the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OutputFormat {
    /// Human-readable text output.
    Text,
    /// Machine-readable JSON output.
    Json,
}

/// Supported command modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    /// Build a plan without publishing crates.
    Plan,
    /// Execute publishable actions.
    Apply,
    /// Resume a previous apply attempt from persisted run artifacts.
    Resume,
    /// Regenerate the umbrella README from catalog plus observed remote state.
    SyncUmbrella,
    /// Yank a published crate version explicitly.
    Yank,
}

/// Selection filter derived from CLI flags.
#[derive(Debug, Clone, Default)]
pub(crate) struct PackageFilter {
    /// Optional package at which catalog processing should start.
    pub from: Option<String>,
    /// Optional package to process exclusively.
    pub only: Option<String>,
}

/// One package entry from the publish wave file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WavePackage {
    /// Cargo package name.
    pub name: String,
    /// Human-facing wave group name.
    pub group: String,
    /// Relative workspace path.
    pub workspace_path: String,
    /// Expected docs.rs URL recorded in the static wave file.
    pub docs_rs: String,
}

/// Normalized publish wave description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PublishWave {
    /// Wave name from `publish-wave.json`.
    pub wave: String,
    /// Ordered packages in the wave.
    pub packages: Vec<WavePackage>,
}

/// Desired public visibility for a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Visibility {
    /// Crate is part of the intended public surface.
    Public,
    /// Crate is internal and should not be treated as public surface.
    Private,
    /// Crate is retired and may require operator review or yanking.
    Retired,
}

/// Docs hosting policy for a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DocsPolicy {
    /// docs.rs is the intended docs surface.
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
pub(crate) enum UmbrellaPolicy {
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
pub(crate) enum CatalogSection {
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

/// One desired-state catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CatalogPackage {
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

/// Desired-state catalog for the docs surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DesiredCatalog {
    /// Schema version.
    pub catalog_version: u32,
    /// Package name of the umbrella crate.
    pub umbrella_package: String,
    /// Catalog entries.
    pub packages: Vec<CatalogPackage>,
}

/// Local workspace package facts used by the planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LocalPackage {
    /// Cargo package name.
    pub name: String,
    /// Parsed local version.
    pub version: Version,
    /// Package manifest path.
    pub manifest_path: PathBuf,
    /// Workspace-relative package path.
    pub workspace_path: PathBuf,
    /// Whether this package has a library or proc-macro target.
    pub has_docs_target: bool,
    /// Readme path from cargo metadata, if any.
    pub readme: Option<String>,
    /// Repository URL from cargo metadata, if any.
    pub repository: Option<String>,
    /// License string from cargo metadata, if any.
    pub license: Option<String>,
    /// Whether Cargo reports the package as publishable.
    pub publish: Option<Vec<String>>,
    /// In-wave local path dependencies.
    pub local_dependencies: Vec<String>,
}

/// Aggregated local workspace state for the selected wave.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkspaceState {
    /// Ordered selected package names.
    pub selected: Vec<String>,
    /// Local package facts across the workspace.
    pub packages: Vec<LocalPackage>,
}

/// High-level registry observation status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryStatus {
    /// The crate is not present remotely.
    Absent,
    /// The crate exists remotely.
    Present,
    /// The registry request failed transiently.
    TemporaryError,
    /// The registry reported a rate limit.
    RateLimited,
    /// The registry request failed due to credentials or authorization.
    AuthError,
    /// The registry response shape was invalid.
    InvalidResponse,
}

/// Source used to obtain registry visibility data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryObservationSource {
    /// The crates.io sparse index.
    Index,
    /// The crates.io HTTP API.
    Api,
}

/// Freshness classification for registry visibility data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryFreshness {
    /// Data was fetched or revalidated during the current run.
    Fresh,
    /// Data came from cache and was not refreshed successfully during the current run.
    Cached,
    /// No usable registry data was available.
    Unavailable,
}

/// Normalized remote registry facts for one package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RegistryObservation {
    /// Package name.
    pub package: String,
    /// Status of the registry observation.
    pub status: RegistryStatus,
    /// Highest visible remote version, if any.
    pub latest_version: Option<Version>,
    /// Whether the exact local version is already present remotely.
    pub exact_version_present: bool,
    /// Source used to obtain the observation.
    pub source: RegistryObservationSource,
    /// Freshness of the observation.
    pub freshness: RegistryFreshness,
    /// RFC3339 timestamp for the observation, if available.
    pub observed_at: Option<String>,
    /// Stable sanitized diagnostic code, if any.
    pub diagnostic_code: Option<String>,
}

impl RegistryObservation {
    /// Returns whether the observation is fresh enough to authorize planner decisions.
    pub(crate) fn is_authoritative(&self) -> bool {
        matches!(self.source, RegistryObservationSource::Index)
            && matches!(self.freshness, RegistryFreshness::Fresh)
    }

    /// Returns whether the exact local version is authoritatively visible in the registry.
    pub(crate) fn exact_version_visible_for_planning(&self) -> bool {
        self.is_authoritative()
            && matches!(self.status, RegistryStatus::Present)
            && self.exact_version_present
    }
}

/// docs.rs availability status for a package version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DocsRsStatus {
    /// docs.rs is not expected for this crate.
    NotExpected,
    /// No published docs are currently visible.
    Absent,
    /// The version appears to be published but docs are not yet live.
    Pending,
    /// The versioned docs page is available.
    Available,
    /// docs.rs indicated a failed build.
    Failed,
    /// docs.rs probe failed temporarily.
    TemporaryError,
}

/// Normalized docs.rs observation for one package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DocsRsObservation {
    /// Package name.
    pub package: String,
    /// Observation status.
    pub status: DocsRsStatus,
    /// Highest visible version on docs.rs, if known.
    pub latest_available_version: Option<Version>,
    /// Whether the exact local version is available.
    pub exact_version_available: bool,
}

/// Planner action for a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanAction {
    /// The exact local version already exists remotely.
    Noop,
    /// The exact local version should be published.
    Publish,
    /// Publishing is blocked on selected upstream workspace packages.
    WaitDependencies,
    /// Publish succeeded earlier but the index is not yet visible to dependents.
    WaitRegistry,
    /// The version is published but docs.rs has not caught up yet.
    WaitDocsRs,
    /// Local source changed since the last released revision but the version did not change.
    NeedsVersionBump,
    /// The umbrella README must be regenerated before publish is meaningful.
    RefreshUmbrella,
    /// A yanking action is explicitly requested.
    Yank,
    /// Human intervention is required before continuing.
    ManualReview,
}

/// Planned outcome for one selected package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlannedPackage {
    /// Package name.
    pub name: String,
    /// Local package version.
    pub local_version: Version,
    /// Highest visible remote version, if any.
    pub remote_version: Option<Version>,
    /// docs.rs status for the package, if relevant.
    pub docs_status: Option<DocsRsStatus>,
    /// Chosen action.
    pub action: PlanAction,
    /// Stable machine-readable reason.
    pub reason: String,
    /// Blocking dependencies inside the selected wave.
    pub blocking_dependencies: Vec<String>,
    /// Whether this is a synthetic planner entry rather than a direct catalog item.
    pub synthetic: bool,
}

/// Aggregated planning result for the current run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Plan {
    /// Schema version for machine-readable artifacts.
    pub schema_version: u32,
    /// Stable run identifier.
    pub run_id: String,
    /// Command mode.
    pub mode: Mode,
    /// Wave name.
    pub wave: String,
    /// Original `--from` filter, if any.
    pub selection_from: Option<String>,
    /// Original `--only` filter, if any.
    pub selection_only: Option<String>,
    /// Prior run id if this run resumed an earlier attempt.
    pub resumed_from_run_id: Option<String>,
    /// Planned package results.
    pub packages: Vec<PlannedPackage>,
}

/// Result for one completed apply action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompletedAction {
    /// Package name.
    pub name: String,
    /// Action that was evaluated.
    pub action: PlanAction,
    /// Stable machine-readable result string.
    pub result: String,
}

/// Result for one blocked package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BlockedAction {
    /// Package name.
    pub name: String,
    /// Blocked action.
    pub action: PlanAction,
    /// Stable machine-readable reason.
    pub reason: String,
}

/// Aggregated apply result for the current run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ApplyResult {
    /// Schema version for machine-readable artifacts.
    pub schema_version: u32,
    /// Stable run identifier.
    pub run_id: String,
    /// Completed actions.
    pub completed_actions: Vec<CompletedAction>,
    /// Blocked actions.
    pub blocked_actions: Vec<BlockedAction>,
    /// Suggested run id to resume next, if the apply stopped in a resumable state.
    pub next_resume_hint: Option<String>,
}

/// Result for umbrella README synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncUmbrellaResult {
    /// Schema version for machine-readable artifacts.
    pub schema_version: u32,
    /// Stable run identifier.
    pub run_id: String,
    /// Whether the README changed on disk.
    pub changed: bool,
    /// Workspace-relative path that was synced.
    pub path: String,
}

/// Persisted local umbrella sync state used by plan/apply to avoid full-catalog fanout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UmbrellaSyncState {
    /// Schema version for machine-readable consumers.
    pub schema_version: u32,
    /// Stable run identifier that produced the sync state.
    pub run_id: String,
    /// RFC3339 timestamp when the sync state was generated.
    pub generated_at: String,
    /// Current git commit when the sync state was produced, if available.
    pub git_commit: Option<String>,
    /// Current README content rendered from the catalog and remote observations.
    pub generated_readme: String,
    /// Full desired-state catalog used to render the README.
    pub catalog: DesiredCatalog,
    /// Full-catalog registry observations used to render the README.
    pub registry: Vec<RegistryObservation>,
    /// Full-catalog docs.rs observations used to render the README.
    pub docs: Vec<DocsRsObservation>,
}

/// Result for an explicit yank request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct YankResult {
    /// Schema version for machine-readable artifacts.
    pub schema_version: u32,
    /// Package name that was yanked.
    pub package: String,
    /// Version that was yanked.
    pub version: Version,
    /// Stable machine-readable result string.
    pub result: String,
}

/// Immutable release fact persisted after successful publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReleaseRecord {
    /// Published version.
    pub version: Version,
    /// Commit checked out when the publish occurred, if known.
    pub git_commit: Option<String>,
    /// Whether the working tree was dirty for the publish.
    pub dirty: bool,
    /// RFC3339 publish timestamp.
    pub published_at: String,
    /// Registry name.
    pub registry: String,
}

/// Release provenance ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReleaseLedger {
    /// Schema version.
    pub schema_version: u32,
    /// Package-to-release history map.
    pub packages: std::collections::BTreeMap<String, Vec<ReleaseRecord>>,
}

/// Summary counts for JSON output and summary artifacts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SummaryCounts {
    /// Number of `noop` packages.
    pub noop: usize,
    /// Number of `publish` packages.
    pub publish: usize,
    /// Number of `wait_dependencies` packages.
    pub wait_dependencies: usize,
    /// Number of `wait_registry` packages.
    pub wait_registry: usize,
    /// Number of `wait_docs_rs` packages.
    pub wait_docs_rs: usize,
    /// Number of `needs_version_bump` packages.
    pub needs_version_bump: usize,
    /// Number of `refresh_umbrella` packages.
    pub refresh_umbrella: usize,
    /// Number of `yank` packages.
    pub yank: usize,
    /// Number of `manual_review` packages.
    pub manual_review: usize,
}
