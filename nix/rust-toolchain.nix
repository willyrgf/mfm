# Shared by development, verification and release packaging.
{ pkgs }:
let
  package = pkgs.rust-bin.stable."1.96.0".minimal.override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
  };
in
{
  inherit package;
  # Child tools must not fall back to host executables or compiler wrappers.
  env = {
    CARGO = "${package}/bin/cargo";
    RUSTC = "${package}/bin/rustc";
    RUSTDOC = "${package}/bin/rustdoc";
    RUSTFMT = "${package}/bin/rustfmt";
    RUSTC_WRAPPER = "";
    RUSTC_WORKSPACE_WRAPPER = "";
  };
}
