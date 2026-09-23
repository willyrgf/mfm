#!/usr/bin/env bash
set -euo pipefail

# A host Cargo subcommand must not replace a supplied Nix executable or its children.
fixture_dir="$(mktemp -d)"
trap 'rm -rf "$fixture_dir"' EXIT
export CARGO_HOME="$fixture_dir/cargo-home"
export CARGO_TARGET_DIR="$fixture_dir/target"
mkdir -p "$CARGO_HOME/bin" "$fixture_dir/src"
for tool in cargo cargo-clippy cargo-fmt cargo-sqlx rustc rustdoc rustfmt clippy-driver; do
  cat > "$CARGO_HOME/bin/$tool" <<'SH'
#!/bin/sh
echo 'unexpected host Rust tool' >&2
exit 99
SH
  chmod +x "$CARGO_HOME/bin/$tool"
done

# Prove the fixture reproduces Cargo's user-bin precedence, even with the pinned PATH.
status=0
cargo clippy --version > "$fixture_dir/shadow.log" 2>&1 || status=$?
test "$status" -eq 99

test -z "$RUSTC_WRAPPER"
test -z "$RUSTC_WORKSPACE_WRAPPER"
test "$CARGO" = "$(command -v cargo)"
test "$RUSTC" = "$(command -v rustc)"
test "$RUSTDOC" = "$(command -v rustdoc)"
test "$RUSTFMT" = "$(command -v rustfmt)"
cargo --version
rustc --version
rustdoc --version
cargo-clippy clippy --version
cargo-fmt fmt --version
cargo-sqlx sqlx --version

cat > "$fixture_dir/Cargo.toml" <<'TOML'
[package]
name = "toolchain-probe"
version = "0.0.0"
edition = "2024"
TOML
cat > "$fixture_dir/src/lib.rs" <<'RS'
/// ```
/// assert_eq!(toolchain_probe::answer(), 42);
/// ```
pub fn answer() -> u32 {
    42
}
RS

cd "$fixture_dir"
cargo check --offline
cargo-clippy clippy --offline -- -D warnings
cargo-fmt fmt -- --check
cargo test --offline --doc
