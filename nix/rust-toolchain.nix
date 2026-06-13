# The single source of MFM's pinned Rust toolchain: consumed by both
# `flake.nix` (the package build) and `nixfied.nix` (the model's tool set),
# so the pin cannot drift between build and verification.
{ pkgs }:
pkgs.rust-bin.stable."1.96.0".minimal.override {
  extensions = [
    "clippy"
    "rustfmt"
  ];
}
