# MFM — `REDESIGN.md` Implementation Plan (Milestone 4)

> Source of truth: `REDESIGN.md` (v4, last updated 2026-02-09).
> Generated: 2026-02-10
>
> Goal: introduce encrypted secret-bearing artifacts (opt-in) so long-running/resumable runs can persist secrets
> safely, while keeping the “no secrets in manifests/events/snapshots/outputs/error details” rule intact.

---

## Summary

Milestone 4 adds a new capability:
- Persist arbitrary secret blobs ONLY as encrypted artifacts.

Non-secret persisted surfaces remain unchanged:
- manifests, events, non-secret artifacts (facts/snapshots/outputs), CLI/API outputs, and error details must never
  contain secrets.

---

## Locked Decisions

- Scope: arbitrary secret blobs (JSON or bytes).
- Secrets are encrypted at rest in the artifact store; ciphertext only is stored.
- Encryption keys are provided at runtime (CLI/API) and never persisted to disk by MFM.
- Errors MUST NOT include plaintext or decrypted bytes.

---

## Deliverables (Step-by-Step)

### M4-01 — Define secret artifact kind + wrapper store

Tasks:
- Add `ArtifactKind::SecretPayload` (if `ArtifactKind` is public, this must be additive and backwards-compatible).
- Implement `SecretArtifactStore` wrapper:
  - `put_secret(bytes) -> ArtifactId` (encrypts, stores ciphertext)
  - `get_secret(id) -> bytes` (decrypts)
  - Underlying `ArtifactStore` sees only ciphertext.

Acceptance:
- Unit tests: roundtrip works; ciphertext differs from plaintext; ciphertext does not contain plaintext substring.

### M4-02 — Key management plumbing (runtime only)

Tasks:
- Define how CLI/API provides the master key (env var or prompt).
- Ensure key material is zeroized when dropped.

Acceptance:
- Tests validate missing key errors are stable and do not leak sensitive data.

### M4-03 — Guardrails: “no plaintext persistence” tests

Tasks:
- Add tests that scan:
  - event store payloads
  - non-secret artifacts
  - snapshots
for known secret strings and assert absence.

Acceptance:
- Regression tests fail if any plaintext secret is persisted.

---

## CI / Done Definition

Milestone 4 is “done” when:
- secrets can be persisted only as encrypted artifacts
- resume/replay can use encrypted artifacts without plaintext leaks
- guardrail tests prevent accidental regressions

