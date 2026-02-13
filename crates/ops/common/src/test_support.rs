//! Shared helpers for ops crate tests.
//!
//! Keep this module limited to non-domain, reusable test wiring helpers.

use std::time::Duration;

use mfm_machine::config::{
    BackoffPolicy, ContextCheckpointing, EventProfile, ExecutionMode, IoMode, RetryPolicy,
    RunConfig,
};

pub fn run_config_live() -> RunConfig {
    run_config_live_with_retry_attempts(1)
}

pub fn run_config_live_with_retry_attempts(max_attempts: u32) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
    }
}

pub fn run_config_live_with_allowlist(prefixes: Vec<String>) -> RunConfig {
    let mut cfg = run_config_live();
    cfg.nix_flake_allowlist = prefixes;
    cfg
}
