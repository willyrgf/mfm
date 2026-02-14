# Repository discovery generator and validator.
{
  pkgs,
  project,
}:

let
  cfg = project.discovery or { };
  enabled = cfg.enable or true;
  strict = cfg.strict or true;
  refreshArg = cfg.refreshArg or "--refresh-discovery";

  defaultRequiredDocs = [
    "README.md"
    "AGENTS.md"
    "CLAUDE.md"
    "ARCHITECTURE.md"
    "REDESIGN.md"
  ];
  requiredDocs = cfg.requiredDocs or defaultRequiredDocs;

  defaultRiskAreas = [
    {
      path = "nixfied/project/conf.nix";
      risk = "Project identity, environment names, and port contract.";
      required_checks = [
        "nix run .#check"
        "nix run .#ci -- --summary"
      ];
    }
    {
      path = "nixfied/project/ci.nix";
      risk = "CI pipeline behavior and release gates.";
      required_checks = [
        "nix run .#ci -- --summary"
      ];
    }
    {
      path = "nixfied/project/quality.nix";
      risk = "Quality checks and discovery drift enforcement.";
      required_checks = [
        "nix run .#check"
      ];
    }
    {
      path = "nixfied/.framework";
      risk = "Framework internals; avoid direct edits in installed repos.";
      required_checks = [
        "nix run .#help"
      ];
    }
  ];
  riskAreas = cfg.riskAreas or defaultRiskAreas;

  requiredDocsJson = builtins.toJSON requiredDocs;
  riskAreasJson = builtins.toJSON riskAreas;

  tool = pkgs.writeShellScriptBin "nixfied-discovery-index" ''
    set -euo pipefail

    MODE="verify"
    ROOT=""

    usage() {
      cat <<'EOF'
Usage: nixfied-discovery-index [--verify|--refresh] [--root PATH]

Modes:
  --verify   Regenerate in memory and fail if committed artifacts drift (default).
  --refresh  Regenerate and write artifacts under docs/.
EOF
    }

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --verify)
          MODE="verify"
          shift
          ;;
        --refresh)
          MODE="refresh"
          shift
          ;;
        --root=*)
          ROOT="''${1#--root=}"
          shift
          ;;
        --root)
          ROOT="''${2:-}"
          if [ -z "$ROOT" ]; then
            echo "ERROR: --root requires a path" >&2
            exit 1
          fi
          shift 2
          ;;
        --help|-h)
          usage
          exit 0
          ;;
        *)
          echo "ERROR: unknown option: $1" >&2
          usage >&2
          exit 1
          ;;
      esac
    done

    if [ -z "$ROOT" ]; then
      ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
    fi
    ROOT=$(${pkgs.coreutils}/bin/realpath "$ROOT")
    cd "$ROOT"

    DOCS_DIR="$ROOT/docs"
    INDEX_PATH="$DOCS_DIR/repo-index.json"
    MAP_PATH="$DOCS_DIR/repo-map.md"

    TMP_DIR=$(${pkgs.coreutils}/bin/mktemp -d)
    TMP_DIR=$(${pkgs.coreutils}/bin/realpath "$TMP_DIR")
    trap 'rm -rf "$TMP_DIR"' EXIT

    TMP_INDEX="$TMP_DIR/repo-index.json"
    TMP_MAP="$TMP_DIR/repo-map.md"

    REQUIRED_DOCS_JSON='${requiredDocsJson}'
    RISK_AREAS_JSON='${riskAreasJson}'
    HAS_GIT_TRACKING="0"
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      HAS_GIT_TRACKING="1"
    fi

    doc_purpose() {
      case "$1" in
        README.md) echo "Primary repository overview and command entrypoints." ;;
        AGENTS.md) echo "Agent instructions and collaboration constraints." ;;
        CLAUDE.md) echo "Additional assistant guidance for this repository." ;;
        ARCHITECTURE.md) echo "Architecture and system design details." ;;
        REDESIGN.md) echo "Redesign notes and migration context." ;;
        *) echo "Project documentation." ;;
      esac
    }

    doc_priority() {
      case "$1" in
        README.md) echo 1 ;;
        AGENTS.md) echo 2 ;;
        CLAUDE.md) echo 3 ;;
        ARCHITECTURE.md) echo 4 ;;
        REDESIGN.md) echo 5 ;;
        *) echo 20 ;;
      esac
    }

    DOCS_TSV="$TMP_DIR/docs.tsv"
    : > "$DOCS_TSV"
    printf '%s\t%s\t%s\t%s\n' \
      "docs/repo-index.json" \
      "Canonical deterministic repository index." \
      "0" \
      "true" >> "$DOCS_TSV"
    printf '%s\t%s\t%s\t%s\n' \
      "docs/repo-map.md" \
      "LLM-facing repository map generated from docs/repo-index.json." \
      "0" \
      "true" >> "$DOCS_TSV"

    doc_present() {
      local doc_path="$1"

      # Ignore untracked local files so discovery stays stable in clean CI checkouts.
      if [ "$HAS_GIT_TRACKING" = "1" ] && ! git ls-files --error-unmatch -- "$doc_path" >/dev/null 2>&1; then
        echo "false"
        return 0
      fi

      if [ -f "$ROOT/$doc_path" ]; then
        echo "true"
      else
        echo "false"
      fi
    }

    while IFS= read -r doc_path; do
      [ -z "$doc_path" ] && continue
      printf '%s\t%s\t%s\t%s\n' \
        "$doc_path" \
        "$(doc_purpose "$doc_path")" \
        "$(doc_priority "$doc_path")" \
        "$(doc_present "$doc_path")" >> "$DOCS_TSV"
    done < <(printf '%s\n' "$REQUIRED_DOCS_JSON" | ${pkgs.jq}/bin/jq -r '.[]')

    DOCS_JSON=$(${pkgs.jq}/bin/jq -R -s '
      split("\n")
      | map(select(length > 0))
      | map(split("\t") | {
          path: .[0],
          purpose: .[1],
          priority: (.[2] | tonumber),
          exists: (.[3] == "true")
        })
      | sort_by(.priority, .path)
    ' "$DOCS_TSV")

    COMPONENTS_TSV="$TMP_DIR/components.tsv"
    : > "$COMPONENTS_TSV"
    while IFS= read -r manifest_path; do
      [ -z "$manifest_path" ] && continue
      rel_path="$manifest_path"
      if [ "$rel_path" = "./flake.nix" ]; then
        rel_path="flake.nix"
      else
        rel_path="''${rel_path#./}"
      fi

      manifest_name=$(${pkgs.coreutils}/bin/basename "$rel_path")
      component_kind="unknown"
      case "$manifest_name" in
        flake.nix) component_kind="nix-flake" ;;
        Cargo.toml) component_kind="rust-cargo" ;;
        package.json) component_kind="node-package" ;;
        go.mod) component_kind="go-module" ;;
        pyproject.toml) component_kind="python-project" ;;
      esac

      component_name="root"
      component_dir=$(${pkgs.coreutils}/bin/dirname "$rel_path")
      if [ "$component_dir" != "." ]; then
        component_name=$(${pkgs.coreutils}/bin/basename "$component_dir")
      fi

      printf '%s\t%s\t%s\n' "$component_name" "$rel_path" "$component_kind" >> "$COMPONENTS_TSV"
    done < <(
      ${pkgs.findutils}/bin/find . -type f \
        \( \
          -name "flake.nix" \
          -o -name "Cargo.toml" \
          -o -name "package.json" \
          -o -name "go.mod" \
          -o -name "pyproject.toml" \
        \) \
        | ${pkgs.coreutils}/bin/sort
    )

    COMPONENTS_JSON=$(${pkgs.jq}/bin/jq -R -s '
      split("\n")
      | map(select(length > 0))
      | map(split("\t") | {
          name: .[0],
          path: .[1],
          kind: .[2]
        })
      | unique_by(.path)
      | sort_by(.path)
    ' "$COMPONENTS_TSV")

    COMMANDS_TSV="$TMP_DIR/commands.tsv"
    : > "$COMMANDS_TSV"
    if [ -d "$ROOT/nixfied/project" ]; then
      while IFS= read -r command_name; do
        [ -z "$command_name" ] && continue
        owner_file=$(
          ${pkgs.ripgrep}/bin/rg -l "commands\\.''${command_name}\\b" "$ROOT/nixfied/project" 2>/dev/null \
            | ${pkgs.coreutils}/bin/head -n 1 || true
        )
        if [ -z "$owner_file" ]; then
          owner_file="nixfied/project"
        else
          owner_file="''${owner_file#"$ROOT"/}"
        fi
        printf '%s\t%s\n' "$command_name" "$owner_file" >> "$COMMANDS_TSV"
      done < <(
        ${pkgs.ripgrep}/bin/rg -o --no-filename 'commands\.[A-Za-z0-9:_-]+' "$ROOT/nixfied/project" 2>/dev/null \
          | ${pkgs.gnused}/bin/sed 's/^commands\.//' \
          | ${pkgs.coreutils}/bin/sort -u
      )
    fi

    COMMANDS_JSON=$(${pkgs.jq}/bin/jq -R -s '
      split("\n")
      | map(select(length > 0))
      | map(split("\t") | {
          name: .[0],
          owner_file: .[1]
        })
      | sort_by(.name)
    ' "$COMMANDS_TSV")

    RISK_AREAS_NORM_JSON=$(printf '%s\n' "$RISK_AREAS_JSON" | ${pkgs.jq}/bin/jq -c '
      map({
        path: .path,
        risk: .risk,
        required_checks: (.required_checks // [])
      })
      | sort_by(.path)
    ')

    ${pkgs.jq}/bin/jq -n \
      --argjson docs "$DOCS_JSON" \
      --argjson components "$COMPONENTS_JSON" \
      --argjson command_surfaces "$COMMANDS_JSON" \
      --argjson risk_areas "$RISK_AREAS_NORM_JSON" \
      '{
        schema_version: 1,
        generated_by: "nixfied-discovery-index",
        docs: $docs,
        components: $components,
        risk_areas: $risk_areas,
        command_surfaces: $command_surfaces
      }' > "$TMP_INDEX"

    {
      echo "# Repository Map"
      echo ""
      echo "Generated from \`docs/repo-index.json\`."
      echo ""
      echo "## Start Here"
      ${pkgs.jq}/bin/jq -r '.docs[] | "- `" + .path + "` - " + .purpose + (if .exists then "" else " (missing)" end)' "$TMP_INDEX"
      echo ""
      echo "## Components"
      ${pkgs.jq}/bin/jq -r '
        if (.components | length) == 0 then
          "- (none detected)"
        else
          .components[] | "- `" + .path + "` (" + .kind + ")"
        end
      ' "$TMP_INDEX"
      echo ""
      echo "## Command Surfaces"
      ${pkgs.jq}/bin/jq -r '
        if (.command_surfaces | length) == 0 then
          "- (none detected)"
        else
          .command_surfaces[] | "- `" + .name + "` from `" + .owner_file + "`"
        end
      ' "$TMP_INDEX"
      echo ""
      echo "## Sensitive Zones"
      ${pkgs.jq}/bin/jq -r '
        if (.risk_areas | length) == 0 then
          "- (none configured)"
        else
          .risk_areas[]
          | "- `" + .path + "` - " + .risk
            + (if (.required_checks | length) > 0 then
                " (checks: " + (.required_checks | join(", ")) + ")"
              else
                ""
              end)
        end
      ' "$TMP_INDEX"
      echo ""
      echo "## Canonical Commands"
      echo '- `nix run .#help`'
      echo '- `nix run .#dev`'
      echo '- `nix run .#test`'
      echo '- `nix run .#build`'
      echo '- `nix run .#check`'
      echo '- `nix run .#ci -- --summary`'
      echo ""
      echo "## Invariants"
      echo '- Treat `nixfied/project/` as the primary customization surface.'
      echo '- Keep command metadata (`api`) aligned with script behavior.'
      echo '- Keep this map and `docs/repo-index.json` in sync via `nix run .#check`.'
    } > "$TMP_MAP"

    verify_file() {
      local generated_path="$1"
      local committed_path="$2"

      if [ ! -f "$committed_path" ]; then
        echo "ERROR: missing discovery artifact path=''${committed_path#"$ROOT"/}" >&2
        return 1
      fi

      if ! ${pkgs.diffutils}/bin/cmp -s "$generated_path" "$committed_path"; then
        echo "ERROR: discovery artifact drift path=''${committed_path#"$ROOT"/}" >&2
        ${pkgs.diffutils}/bin/diff -u "$committed_path" "$generated_path" >&2 || true
        return 1
      fi

      echo "OK: discovery artifact current path=''${committed_path#"$ROOT"/}"
      return 0
    }

    write_if_changed() {
      local generated_path="$1"
      local committed_path="$2"

      if [ -f "$committed_path" ] && ${pkgs.diffutils}/bin/cmp -s "$generated_path" "$committed_path"; then
        echo "OK: discovery artifact unchanged path=''${committed_path#"$ROOT"/}"
        return 0
      fi

      ${pkgs.coreutils}/bin/mkdir -p "$(${pkgs.coreutils}/bin/dirname "$committed_path")"
      ${pkgs.coreutils}/bin/cp "$generated_path" "$committed_path"
      echo "INFO: wrote discovery artifact path=''${committed_path#"$ROOT"/}"
      return 0
    }

    if [ "$MODE" = "refresh" ]; then
      write_if_changed "$TMP_INDEX" "$INDEX_PATH"
      write_if_changed "$TMP_MAP" "$MAP_PATH"
      echo "OK: discovery refresh complete root=$ROOT"
      exit 0
    fi

    rc=0
    verify_file "$TMP_INDEX" "$INDEX_PATH" || rc=1
    verify_file "$TMP_MAP" "$MAP_PATH" || rc=1

    if [ "$rc" -ne 0 ]; then
      echo "ERROR: discovery artifacts are out of date." >&2
      echo "INFO: refresh with: nix run .#check -- ${refreshArg}" >&2
      exit "$rc"
    fi

    echo "OK: discovery artifacts are current"
  '';
in
{
  inherit
    enabled
    strict
    refreshArg
    requiredDocs
    riskAreas
    tool
    ;
}
