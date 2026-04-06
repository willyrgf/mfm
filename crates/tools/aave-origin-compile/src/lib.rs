#![warn(missing_docs)]

//! Thin compile adapter for turning fetched Aave Origin sources into a generic
//! compiled contract-set manifest.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use mfm_evm_runtime::contract_set::{
    validate_compiled_contract_set_manifest, CompiledContractSetEntry, CompiledContractSetManifest,
    COMPILED_CONTRACT_SET_KIND,
};
use mfm_evm_runtime::dcv::{AbiJson, BytecodeJson, ContractArtifactConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tempfile::TempDir;
use thiserror::Error;

const SOURCE_KIND: &str = "aave_v3_origin_source_v1";
const DEFAULT_REPO_URL: &str = "https://github.com/aave-dao/aave-v3-origin";
const DEFAULT_COMMIT_SHA: &str = "1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978";
const DEFAULT_SOLC_PATH: &str = "solc";

const TOKEN_ARTIFACT_PATH: &str = "out/TestnetERC20.sol/TestnetERC20.json";
const POOL_ARTIFACT_PATH: &str = "out/Pool.sol/Pool.json";
const A_TOKEN_ARTIFACT_PATH: &str = "out/AToken.sol/AToken.json";
const VARIABLE_DEBT_ARTIFACT_PATH: &str = "out/VariableDebtToken.sol/VariableDebtToken.json";

#[derive(Clone, Debug, PartialEq, Eq)]
/// Environment-driven configuration for the compile adapter.
pub struct CompileConfig {
    expected_repo_url: String,
    expected_commit_sha: String,
    solc_path: String,
}

impl CompileConfig {
    /// Builds a config from the standard Aave Origin environment overrides.
    pub fn from_env() -> Self {
        Self {
            expected_repo_url: std::env::var("MFM_AAVE_V3_ORIGIN_EXPECTED_REPO_URL")
                .unwrap_or_else(|_| DEFAULT_REPO_URL.to_string()),
            expected_commit_sha: std::env::var("MFM_AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA")
                .unwrap_or_else(|_| DEFAULT_COMMIT_SHA.to_string()),
            solc_path: std::env::var("MFM_AAVE_V3_ORIGIN_SOLC_PATH")
                .unwrap_or_else(|_| DEFAULT_SOLC_PATH.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
/// Normalized source metadata emitted by the fetch step.
struct OriginSourceMetadata {
    repo_url: String,
    commit_sha: String,
    local_path: String,
}

#[derive(Clone, Debug, Deserialize)]
struct OriginSourceManifest {
    kind: String,
    source: OriginSourceMetadata,
}

struct CompiledArtifacts {
    token: ContractArtifactConfig,
    pool: ContractArtifactConfig,
    a_token: ContractArtifactConfig,
    variable_debt: ContractArtifactConfig,
}

#[derive(Debug, Error)]
/// Errors returned while validating input, compiling Aave Origin, or serializing output.
pub enum CompileError {
    /// Stdin was empty.
    #[error("stdin was empty")]
    EmptyStdin,
    /// Stdin could not be read.
    #[error("failed to read stdin: {0}")]
    ReadStdin(std::io::Error),
    /// Input JSON was malformed.
    #[error("failed to parse compile input json: {0}")]
    ParseInput(serde_json::Error),
    /// The input `kind` was not the expected fetch manifest kind.
    #[error("compile input kind `{0}` is unsupported")]
    UnsupportedInputKind(String),
    /// The fetch manifest did not include `source.local_path`.
    #[error("compile input source.local_path is required")]
    MissingLocalPath,
    /// The fetch manifest did not match the pinned upstream repo/commit pair.
    #[error("compile input source {repo_url}@{commit_sha} is unsupported by the current Aave Origin backend")]
    UnsupportedSource {
        /// Repo URL from the rejected manifest.
        repo_url: String,
        /// Commit SHA from the rejected manifest.
        commit_sha: String,
    },
    /// The fetched source path did not exist on disk.
    #[error("source path `{0}` does not exist")]
    MissingSourcePath(String),
    /// The temporary compile workspace could not be created.
    #[error("failed to create temporary workspace: {0}")]
    CreateTempDir(std::io::Error),
    /// A required external command failed.
    #[error("{program} failed: {message}")]
    CommandFailed {
        /// Program name that failed.
        program: &'static str,
        /// Human-readable failure summary.
        message: String,
    },
    /// A required file could not be written.
    #[error("failed to write `{path}`: {source}")]
    WriteFile {
        /// File path that failed to write.
        path: String,
        /// Underlying IO error.
        source: std::io::Error,
    },
    /// A required file could not be read.
    #[error("failed to read `{path}`: {source}")]
    ReadFile {
        /// File path that failed to read.
        path: String,
        /// Underlying IO error.
        source: std::io::Error,
    },
    /// A Foundry artifact file did not contain valid JSON.
    #[error("artifact `{path}` was invalid json: {source}")]
    ParseArtifactJson {
        /// Artifact path that failed to parse.
        path: String,
        /// Underlying parse error.
        source: serde_json::Error,
    },
    /// A required field was missing from a Foundry artifact.
    #[error("artifact `{path}` was missing `{field}`")]
    MissingArtifactField {
        /// Artifact path missing the field.
        path: String,
        /// Missing field name.
        field: &'static str,
    },
    /// The emitted generic manifest failed runtime validation.
    #[error("compiled contract-set manifest was invalid: {0}")]
    InvalidManifest(String),
    /// The final manifest could not be serialized.
    #[error("failed to serialize compile output: {0}")]
    SerializeOutput(serde_json::Error),
}

/// Reads the full stdin payload and rejects empty input.
pub fn read_stdin_string() -> Result<String, CompileError> {
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .map_err(CompileError::ReadStdin)?;
    if stdin.trim().is_empty() {
        return Err(CompileError::EmptyStdin);
    }
    Ok(stdin)
}

/// Validates fetch output, runs `forge build`, and returns the generic compile manifest.
pub fn compile_from_stdin(
    input_json: &str,
    cfg: &CompileConfig,
) -> Result<CompiledContractSetManifest, CompileError> {
    let source = parse_source_manifest(input_json, cfg)?;
    let workspace = prepare_origin_workspace(&source, cfg)?;
    let origin_dir = workspace.path().join("origin");
    let artifacts = compile_contract_artifacts(&origin_dir)?;
    build_compile_manifest(source, artifacts)
}

fn parse_source_manifest(
    input_json: &str,
    cfg: &CompileConfig,
) -> Result<OriginSourceMetadata, CompileError> {
    let manifest: OriginSourceManifest =
        serde_json::from_str(input_json).map_err(CompileError::ParseInput)?;
    if manifest.kind != SOURCE_KIND {
        return Err(CompileError::UnsupportedInputKind(manifest.kind));
    }
    if manifest.source.local_path.trim().is_empty() {
        return Err(CompileError::MissingLocalPath);
    }
    if manifest.source.repo_url != cfg.expected_repo_url
        || manifest.source.commit_sha != cfg.expected_commit_sha
    {
        return Err(CompileError::UnsupportedSource {
            repo_url: manifest.source.repo_url,
            commit_sha: manifest.source.commit_sha,
        });
    }
    if !Path::new(&manifest.source.local_path).exists() {
        return Err(CompileError::MissingSourcePath(manifest.source.local_path));
    }
    Ok(manifest.source)
}

fn build_compile_manifest(
    _source: OriginSourceMetadata,
    artifacts: CompiledArtifacts,
) -> Result<CompiledContractSetManifest, CompileError> {
    let contracts = vec![
        CompiledContractSetEntry {
            id: "usdc".to_string(),
            artifact: artifacts.token.clone(),
            constructor_args: vec![json!("USDX"), json!("USDX"), json!(6)],
        },
        CompiledContractSetEntry {
            id: "wbtc".to_string(),
            artifact: artifacts.token,
            constructor_args: vec![json!("WBTC"), json!("WBTC"), json!(8)],
        },
        CompiledContractSetEntry {
            id: "pool".to_string(),
            artifact: artifacts.pool,
            constructor_args: vec![],
        },
        CompiledContractSetEntry {
            id: "usdc_a_token".to_string(),
            artifact: artifacts.a_token.clone(),
            constructor_args: vec![],
        },
        CompiledContractSetEntry {
            id: "wbtc_a_token".to_string(),
            artifact: artifacts.a_token,
            constructor_args: vec![],
        },
        CompiledContractSetEntry {
            id: "usdc_variable_debt_token".to_string(),
            artifact: artifacts.variable_debt.clone(),
            constructor_args: vec![],
        },
        CompiledContractSetEntry {
            id: "wbtc_variable_debt_token".to_string(),
            artifact: artifacts.variable_debt,
            constructor_args: vec![],
        },
    ];
    validate_compiled_contract_set_manifest(&CompiledContractSetManifest {
        kind: COMPILED_CONTRACT_SET_KIND.to_string(),
        contracts: contracts.clone(),
    })
    .map_err(|err| CompileError::InvalidManifest(err.info.message))?;

    Ok(CompiledContractSetManifest {
        kind: COMPILED_CONTRACT_SET_KIND.to_string(),
        contracts,
    })
}

/// Serializes the compile manifest as canonical JSON for stdout emission.
pub fn serialize_compile_manifest(
    manifest: &CompiledContractSetManifest,
) -> Result<String, CompileError> {
    serde_json::to_string(manifest).map_err(CompileError::SerializeOutput)
}

fn prepare_origin_workspace(
    source: &OriginSourceMetadata,
    cfg: &CompileConfig,
) -> Result<TempDir, CompileError> {
    let workspace = tempfile::tempdir().map_err(CompileError::CreateTempDir)?;
    let origin_dir = workspace.path().join("origin");
    std::fs::create_dir_all(&origin_dir).map_err(|source| CompileError::WriteFile {
        path: origin_dir.display().to_string(),
        source,
    })?;

    run_command(
        "cp",
        &[
            "-R",
            &format!("{}/.", source.local_path),
            &origin_dir.display().to_string(),
        ],
        None,
    )?;
    run_command(
        "chmod",
        &["-R", "u+w", &origin_dir.display().to_string()],
        None,
    )?;
    write_foundry_toml(&origin_dir, &cfg.solc_path)?;

    Ok(workspace)
}

fn compile_contract_artifacts(origin_dir: &Path) -> Result<CompiledArtifacts, CompileError> {
    run_command("forge", &["build", "--quiet"], Some(origin_dir))?;

    Ok(CompiledArtifacts {
        token: read_contract_artifact(origin_dir, TOKEN_ARTIFACT_PATH)?,
        pool: read_contract_artifact(origin_dir, POOL_ARTIFACT_PATH)?,
        a_token: read_contract_artifact(origin_dir, A_TOKEN_ARTIFACT_PATH)?,
        variable_debt: read_contract_artifact(origin_dir, VARIABLE_DEBT_ARTIFACT_PATH)?,
    })
}

fn write_foundry_toml(origin_dir: &Path, solc_path: &str) -> Result<(), CompileError> {
    let foundry_toml = format!(
        r#"[profile.default]
src = "src"
test = "tests"
script = "scripts"
optimizer = true
optimizer_runs = 200
solc = "{solc_path}"
evm_version = "shanghai"
bytecode_hash = "none"
out = "out"
libs = ["lib"]
remappings = []
fs_permissions = [
  {{ access = "write", path = "./reports" }},
  {{ access = "read", path = "./out" }},
  {{ access = "read", path = "./config" }},
]
ffi = true
"#
    );

    let path = origin_dir.join("foundry.toml");
    std::fs::write(&path, foundry_toml).map_err(|source| CompileError::WriteFile {
        path: path.display().to_string(),
        source,
    })
}

fn read_contract_artifact(
    origin_dir: &Path,
    relative_path: &'static str,
) -> Result<ContractArtifactConfig, CompileError> {
    let path = origin_dir.join(relative_path);
    let bytes = std::fs::read(&path).map_err(|source| CompileError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| CompileError::ParseArtifactJson {
            path: path.display().to_string(),
            source,
        })?;
    artifact_from_forge_json(&value, path)
}

fn artifact_from_forge_json(
    value: &Value,
    path: PathBuf,
) -> Result<ContractArtifactConfig, CompileError> {
    let abi = value
        .get("abi")
        .cloned()
        .ok_or_else(|| CompileError::MissingArtifactField {
            path: path.display().to_string(),
            field: "abi",
        })?;
    let bytecode = value
        .pointer("/bytecode/object")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| CompileError::MissingArtifactField {
            path: path.display().to_string(),
            field: "bytecode.object",
        })?;

    Ok(ContractArtifactConfig {
        abi: AbiJson::from(abi),
        bytecode: BytecodeJson::from(json!({ "object": bytecode })),
    })
}

fn run_command(
    program: &'static str,
    args: &[&str],
    current_dir: Option<&Path>,
) -> Result<(), CompileError> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let output = command
        .output()
        .map_err(|err| CompileError::CommandFailed {
            program,
            message: err.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let message = if stderr.is_empty() {
        format!("exit status {}", output.status)
    } else {
        stderr
    };
    Err(CompileError::CommandFailed { program, message })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> CompileConfig {
        CompileConfig {
            expected_repo_url: DEFAULT_REPO_URL.to_string(),
            expected_commit_sha: DEFAULT_COMMIT_SHA.to_string(),
            solc_path: DEFAULT_SOLC_PATH.to_string(),
        }
    }

    fn source_manifest_json(repo_url: &str, commit_sha: &str, local_path: &str) -> String {
        serde_json::json!({
            "kind": SOURCE_KIND,
            "source": {
                "repo_url": repo_url,
                "commit_sha": commit_sha,
                "local_path": local_path,
            }
        })
        .to_string()
    }

    fn fake_artifact(bytecode_object: &str) -> ContractArtifactConfig {
        ContractArtifactConfig {
            abi: AbiJson::from(json!([])),
            bytecode: BytecodeJson::from(json!({ "object": bytecode_object })),
        }
    }

    #[test]
    fn parse_source_manifest_rejects_unsupported_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = parse_source_manifest(
            &source_manifest_json(
                "https://example.com/other",
                DEFAULT_COMMIT_SHA,
                &dir.path().display().to_string(),
            ),
            &test_config(),
        )
        .expect_err("must fail");

        match err {
            CompileError::UnsupportedSource { .. } => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn parse_source_manifest_rejects_wrong_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = parse_source_manifest(
            &serde_json::json!({
                "kind": "unexpected",
                "source": {
                    "repo_url": DEFAULT_REPO_URL,
                    "commit_sha": DEFAULT_COMMIT_SHA,
                    "local_path": dir.path().display().to_string(),
                }
            })
            .to_string(),
            &test_config(),
        )
        .expect_err("must fail");

        match err {
            CompileError::UnsupportedInputKind(kind) => assert_eq!(kind, "unexpected"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn build_compile_manifest_emits_expected_contract_ids_and_args() {
        let manifest = build_compile_manifest(
            OriginSourceMetadata {
                repo_url: DEFAULT_REPO_URL.to_string(),
                commit_sha: DEFAULT_COMMIT_SHA.to_string(),
                local_path: "/nix/store/example".to_string(),
            },
            CompiledArtifacts {
                token: fake_artifact("0x6000"),
                pool: fake_artifact("0x6001"),
                a_token: fake_artifact("0x6002"),
                variable_debt: fake_artifact("0x6003"),
            },
        )
        .expect("manifest");

        assert_eq!(manifest.kind, COMPILED_CONTRACT_SET_KIND);
        let ids = manifest
            .contracts
            .iter()
            .map(|contract| contract.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "usdc",
                "wbtc",
                "pool",
                "usdc_a_token",
                "wbtc_a_token",
                "usdc_variable_debt_token",
                "wbtc_variable_debt_token",
            ]
        );
        assert_eq!(
            manifest.contracts[0].constructor_args,
            vec![json!("USDX"), json!("USDX"), json!(6)]
        );
        assert_eq!(
            manifest.contracts[1].constructor_args,
            vec![json!("WBTC"), json!("WBTC"), json!(8)]
        );
        assert!(manifest.contracts[2..]
            .iter()
            .all(|contract| contract.constructor_args.is_empty()));
    }

    #[test]
    fn artifact_from_forge_json_requires_bytecode_object() {
        let err = artifact_from_forge_json(
            &json!({
                "abi": []
            }),
            PathBuf::from("/tmp/token.json"),
        )
        .expect_err("must fail");

        match err {
            CompileError::MissingArtifactField { field, .. } => {
                assert_eq!(field, "bytecode.object");
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
