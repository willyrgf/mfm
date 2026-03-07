#![warn(missing_docs)]
//! Shared workspace integration and parity test helpers.
//!
//! This crate keeps cross-workspace test plumbing in one place so parity suites can coordinate
//! run ids and shared setup without duplicating helper code in every test binary.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_integration_tests::parity_run_ids::read_parity_evm_run_id;
//!
//! let _run_id = read_parity_evm_run_id();
//! ```

use std::path::{Path, PathBuf};

use mfm_machine::ids::RunId;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Helpers for persisting parity run ids between coordinated integration-test phases.
pub mod parity_run_ids {
    use super::*;

    /// Environment variable that points to the handoff file for EVM parity run ids.
    pub const PARITY_EVM_RUN_IDS_PATH_ENV: &str = "MFM_PARITY_EVM_RETH_RUN_IDS_PATH";
    /// Environment variable that points to the handoff file for Aave parity run ids.
    pub const PARITY_AAVE_RUN_IDS_PATH_ENV: &str = "MFM_PARITY_AAVE_V3_RUN_IDS_PATH";

    const PARITY_EVM_RUN_IDS_KIND: &str = "parity_evm_reth_run_ids_v1";
    const PARITY_AAVE_RUN_IDS_KIND: &str = "parity_aave_v3_run_ids_v1";

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ParityEvmRunIds {
        kind: String,
        evm_run_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ParityAaveRunIds {
        kind: String,
        phase_a_run_id: String,
        phase_b_run_id: String,
    }

    /// Writes a single EVM parity run id to the configured handoff file.
    pub fn write_parity_evm_run_id(run_id: &RunId) {
        let path = required_env_path(PARITY_EVM_RUN_IDS_PATH_ENV);
        let payload = ParityEvmRunIds {
            kind: PARITY_EVM_RUN_IDS_KIND.to_string(),
            evm_run_id: run_id.0.to_string(),
        };
        write_json_atomic(&path, &payload, "evm parity run ids");
    }

    /// Writes the phase A and phase B Aave parity run ids to the configured handoff file.
    pub fn write_parity_aave_run_ids(phase_a_run_id: &RunId, phase_b_run_id: &RunId) {
        let path = required_env_path(PARITY_AAVE_RUN_IDS_PATH_ENV);
        let payload = ParityAaveRunIds {
            kind: PARITY_AAVE_RUN_IDS_KIND.to_string(),
            phase_a_run_id: phase_a_run_id.0.to_string(),
            phase_b_run_id: phase_b_run_id.0.to_string(),
        };
        write_json_atomic(&path, &payload, "aave parity run ids");
    }

    /// Reads the EVM parity run id from the configured handoff file.
    pub fn read_parity_evm_run_id() -> RunId {
        let path = required_env_path(PARITY_EVM_RUN_IDS_PATH_ENV);
        let payload: ParityEvmRunIds = read_json(&path, "evm parity run ids");
        assert_eq!(
            payload.kind,
            PARITY_EVM_RUN_IDS_KIND,
            "unexpected kind for evm parity run ids file `{}`",
            path.display()
        );
        parse_run_id("evm_run_id", &payload.evm_run_id)
    }

    /// Reads the phase A and phase B Aave parity run ids from the configured handoff file.
    pub fn read_parity_aave_run_ids() -> (RunId, RunId) {
        let path = required_env_path(PARITY_AAVE_RUN_IDS_PATH_ENV);
        let payload: ParityAaveRunIds = read_json(&path, "aave parity run ids");
        assert_eq!(
            payload.kind,
            PARITY_AAVE_RUN_IDS_KIND,
            "unexpected kind for aave parity run ids file `{}`",
            path.display()
        );
        let phase_a_run_id = parse_run_id("phase_a_run_id", &payload.phase_a_run_id);
        let phase_b_run_id = parse_run_id("phase_b_run_id", &payload.phase_b_run_id);
        (phase_a_run_id, phase_b_run_id)
    }

    fn required_env_path(env_var: &str) -> PathBuf {
        let path = std::env::var(env_var)
            .unwrap_or_else(|_| panic!("`{env_var}` must be set for parity run-id handoff"));
        PathBuf::from(path)
    }

    fn write_json_atomic<T: Serialize>(path: &Path, value: &T, label: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|err| {
                panic!(
                    "failed creating parent directory `{}` for {label}: {err}",
                    parent.display()
                )
            });
        }

        let bytes = serde_json::to_vec_pretty(value)
            .unwrap_or_else(|err| panic!("failed serializing {label}: {err}"));
        let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));

        std::fs::write(&tmp_path, bytes).unwrap_or_else(|err| {
            panic!(
                "failed writing temp file `{}` for {label}: {err}",
                tmp_path.display()
            )
        });

        std::fs::rename(&tmp_path, path).unwrap_or_else(|err| {
            panic!(
                "failed moving temp file `{}` to `{}` for {label}: {err}",
                tmp_path.display(),
                path.display()
            )
        });
    }

    fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> T {
        let bytes = std::fs::read(path).unwrap_or_else(|err| {
            panic!("failed reading {label} file `{}`: {err}", path.display())
        });
        serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            panic!("failed decoding {label} file `{}`: {err}", path.display())
        })
    }

    fn parse_run_id(field: &str, value: &str) -> RunId {
        let parsed = uuid::Uuid::parse_str(value)
            .unwrap_or_else(|err| panic!("invalid uuid in `{field}` (`{value}`): {err}"));
        RunId(parsed)
    }
}
