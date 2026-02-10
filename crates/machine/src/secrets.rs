//! Secrets scanning and redaction helpers (Milestone 1).
//!
//! Source of truth: `REDESIGN.md` (v4).
//!
//! Milestone 1 hard rule: secrets must not appear in persisted surfaces:
//! - manifests, events, artifacts (including fact payloads and context snapshots), CLI/API outputs, and error details.
//!
//! This module provides:
//! - conservative secret detection for JSON payloads (by key name, and a lightweight mnemonic heuristic)
//! - redaction for error messages/details before persistence

use crate::errors::ErrorInfo;

// Keep this list intentionally small and high-signal to avoid false positives.
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "password",
    "passphrase",
    "mnemonic",
    "private_key",
    "privatekey",
    // Avoid matching the generic word "seed" to reduce false positives.
    "seed phrase",
    "seed_phrase",
    "seedphrase",
    "api_key",
    "apikey",
    "x-api-key",
    "x_api_key",
    "access_key",
    "accesskey",
    "secret_key",
    "secretkey",
    "aws_access_key_id",
    "aws_secret_access_key",
    "access_token",
    "refresh_token",
    "id_token",
    "authorization",
    "bearer ",
];

pub(crate) fn string_contains_secrets(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if FORBIDDEN_SUBSTRINGS.iter().any(|pat| lower.contains(pat)) {
        return true;
    }

    // Lightweight heuristic for BIP39-like mnemonics:
    // - 12..=24 whitespace-separated words
    // - each word is lowercase ASCII letters only
    looks_like_mnemonic_phrase(s)
}

pub(crate) fn json_contains_secrets(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
        serde_json::Value::String(s) => string_contains_secrets(s),
        serde_json::Value::Array(a) => a.iter().any(json_contains_secrets),
        serde_json::Value::Object(m) => {
            // Check keys first; this catches the common "mnemonic"/"private_key" cases.
            if m.keys().any(|k| string_contains_secrets(k)) {
                return true;
            }
            m.values().any(json_contains_secrets)
        }
    }
}

pub(crate) fn redact_error_info(info: &mut ErrorInfo) {
    // Details are always dropped in Milestone 1; if something needs to be persisted, it must be
    // explicitly modeled as a non-secret reference.
    info.details = None;

    if string_contains_secrets(&info.message) {
        info.message = "error details redacted".to_string();
    }
}

fn looks_like_mnemonic_phrase(s: &str) -> bool {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() < 12 || words.len() > 24 {
        return false;
    }

    for w in words {
        if w.is_empty() {
            return false;
        }
        if !w.chars().all(|c| c.is_ascii_lowercase()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forbidden_substrings() {
        assert!(string_contains_secrets("password=123"));
        assert!(string_contains_secrets("MNEMONIC"));
        assert!(string_contains_secrets("private_key"));
        assert!(string_contains_secrets("seed phrase: ..."));
        assert!(string_contains_secrets("AWS_SECRET_ACCESS_KEY=..."));
        assert!(!string_contains_secrets("artifact_written"));
        assert!(!string_contains_secrets("seeding_database"));
    }

    #[test]
    fn detects_mnemonic_phrase_like_strings() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(string_contains_secrets(phrase));
    }

    #[test]
    fn json_detection_is_recursive_and_checks_keys() {
        let v = serde_json::json!({
            "ok": {"nested": [1, 2]},
            "mnemonic": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        });
        assert!(json_contains_secrets(&v));

        let v = serde_json::json!({"ok": {"nested": [1, 2]}});
        assert!(!json_contains_secrets(&v));
    }
}
