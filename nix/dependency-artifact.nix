{
  lib,
  pkgs,
  rustPlatform,
  src,
}:
let
  sourceFilterVersion = "dependency-inputs-v1";

  relativePath = source: path: lib.removePrefix "${toString source}/" (toString path);

  isDocumentation =
    relative:
    let
      fileName = builtins.baseNameOf relative;
    in
    relative == "docs" || lib.hasPrefix "docs/" relative || lib.hasSuffix ".md" fileName;

  sourceFilter =
    source: path: type:
    let
      relative = relativePath source path;
      isExcludedDirectory =
        relative == "target"
        || lib.hasPrefix "target/" relative
        || relative == ".git"
        || lib.hasPrefix ".git/" relative
        || isDocumentation relative;
      fileName = builtins.baseNameOf relative;
    in
    if type == "directory" then
      !isExcludedDirectory
    else
      !isDocumentation relative
      && (
        fileName == "Cargo.toml"
        || fileName == "Cargo.lock"
        || fileName == "build.rs"
        || fileName == "dependency-artifact-stubs.py"
        || !lib.hasSuffix ".rs" fileName
      );

  filteredSource =
    source:
    lib.cleanSourceWith {
      name = "mfm-${sourceFilterVersion}";
      src = source;
      filter = sourceFilter source;
    };

  dependencySource = filteredSource src;

  mkFixture =
    {
      name,
      manifest,
      rustSource,
      buildScript,
      compileData,
      documentation,
    }:
    pkgs.runCommand "mfm-dependency-source-fixture-${name}" { } ''
      mkdir -p "$out/src" "$out/data" "$out/docs"
      printf '%s' ${lib.escapeShellArg manifest} > "$out/Cargo.toml"
      printf '%s' ${lib.escapeShellArg rustSource} > "$out/src/lib.rs"
      printf '%s' ${lib.escapeShellArg buildScript} > "$out/build.rs"
      printf '%s' ${lib.escapeShellArg compileData} > "$out/data/input.json"
      printf '%s' ${lib.escapeShellArg documentation} > "$out/README.md"
      printf '%s' ${lib.escapeShellArg documentation} > "$out/docs/notes.txt"
    '';

  baseFixture = mkFixture {
    name = "base";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn unchanged() {}\n";
    buildScript = "fn main() {}\n";
    compileData = "{\"value\":1}\n";
    documentation = "documentation\n";
  };
  documentationFixture = mkFixture {
    name = "documentation";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn unchanged() {}\n";
    buildScript = "fn main() {}\n";
    compileData = "{\"value\":1}\n";
    documentation = "documentation changed\n";
  };
  rustFixture = mkFixture {
    name = "rust";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn changed() {}\n";
    buildScript = "fn main() {}\n";
    compileData = "{\"value\":1}\n";
    documentation = "documentation\n";
  };
  manifestFixture = mkFixture {
    name = "manifest";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.2.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn unchanged() {}\n";
    buildScript = "fn main() {}\n";
    compileData = "{\"value\":1}\n";
    documentation = "documentation\n";
  };
  buildFixture = mkFixture {
    name = "build";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn unchanged() {}\n";
    buildScript = "fn main() { println!(\"changed\"); }\n";
    compileData = "{\"value\":1}\n";
    documentation = "documentation\n";
  };
  dataFixture = mkFixture {
    name = "data";
    manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
    rustSource = "pub fn unchanged() {}\n";
    buildScript = "fn main() {}\n";
    compileData = "{\"value\":2}\n";
    documentation = "documentation\n";
  };

  sourceInputRegressionTest =
    pkgs.runCommand "mfm-dependency-source-input-regression"
      {
        baseSource = filteredSource baseFixture;
        documentationSource = filteredSource documentationFixture;
        rustSource = filteredSource rustFixture;
        manifestSource = filteredSource manifestFixture;
        buildSource = filteredSource buildFixture;
        dataSource = filteredSource dataFixture;
      }
      ''
        set -euo pipefail
        snapshot() {
          (
            cd "$1"
            find . -type f -print0 | sort -z | xargs -0 sha256sum
          )
        }

        test "$(snapshot "$baseSource")" = "$(snapshot "$documentationSource")"
        test "$(snapshot "$baseSource")" = "$(snapshot "$rustSource")"
        test "$(snapshot "$baseSource")" != "$(snapshot "$manifestSource")"
        test "$(snapshot "$baseSource")" != "$(snapshot "$buildSource")"
        test "$(snapshot "$baseSource")" != "$(snapshot "$dataSource")"
        test -f "$baseSource/Cargo.toml"
        test -f "$baseSource/build.rs"
        test -f "$baseSource/data/input.json"
        test ! -e "$baseSource/README.md"
        test ! -e "$baseSource/docs/notes.txt"
        test ! -e "$baseSource/src/lib.rs"

        cat > "$out" <<'EOF'
        dependency source-input regression tests passed
        documentation and local Rust edits are excluded;
        manifest, build-script, and generic compile-data edits are retained.
        EOF
      '';

  currentSourceRegressionTest =
    pkgs.runCommand "mfm-dependency-source-inventory"
      {
        inherit dependencySource;
      }
      ''
        set -euo pipefail
        test -f "$dependencySource/Cargo.toml"
        test -f "$dependencySource/Cargo.lock"
        test -f "$dependencySource/nix/dependency-artifact-stubs.py"
        if find "$dependencySource" -type f -name '*.rs' ! -name 'build.rs' -print -quit | grep -q .; then
          echo "dependency source unexpectedly contains workspace Rust" >&2
          exit 1
        fi
        if find "$dependencySource" -type f -name '*.md' -print -quit | grep -q .; then
          echo "dependency source unexpectedly contains documentation" >&2
          exit 1
        fi
        printf '%s\n' \
          "source-filter=${sourceFilterVersion}" \
          "cargo-manifests=$(find "$dependencySource" -type f -name 'Cargo.toml' | wc -l)" \
          "build-scripts=$(find "$dependencySource" -type f -name 'build.rs' | wc -l)" > "$out"
      '';

  sourceInputTests =
    pkgs.runCommand "mfm-dependency-artifact-source-tests"
      {
        inherit sourceInputRegressionTest currentSourceRegressionTest;
      }
      ''
        set -euo pipefail
        cat "$sourceInputRegressionTest" "$currentSourceRegressionTest" > "$out"
      '';

  dependencyArtifact = rustPlatform.buildRustPackage {
    pname = "mfm-dependency-artifact";
    version = "0.1.29";
    src = dependencySource;
    cargoLock.lockFile = "${dependencySource}/Cargo.lock";
    nativeBuildInputs = [ pkgs.python3 ];
    cargoBuildFlags = [
      "--workspace"
      "--lib"
      "--all-features"
    ];
    buildType = "debug";
    doCheck = false;
    dontStrip = true;
    CARGO_INCREMENTAL = "0";
    CARGO_PROFILE_DEV_DEBUG = "1";
    CARGO_PROFILE_DEV_SPLIT_DEBUGINFO = "off";
    RUST_BACKTRACE = "1";
    preBuild = ''
      python3 nix/dependency-artifact-stubs.py "$PWD"
    '';
    installPhase = ''
      runHook preInstall
      dependencyDir="$(find target -type d -path '*/debug/deps' -print -quit)"
      if [ -z "$dependencyDir" ]; then
        echo "dependency artifact did not produce a debug dependency directory" >&2
        exit 1
      fi

      mkdir -p "$out/dependency-artifacts" "$out/metadata"
      cp -a "$dependencyDir" "$out/dependency-artifacts/deps"
      bytes="$(du -sb "$dependencyDir" | cut -f1)"
      files="$(find "$dependencyDir" -type f | wc -l)"
      cat > "$out/metadata/pilot.json" <<EOF
      {"kind":"mfm-nix-dependency-artifact","source_filter":"${sourceFilterVersion}","profile":"dev","features":"all","workspace_libraries":true,"local_sources":"generated-stubs","bytes":''${bytes},"files":''${files}}
      EOF
      runHook postInstall
    '';
  };
in
{
  inherit
    dependencyArtifact
    dependencySource
    sourceInputTests
    sourceFilterVersion
    ;
}
