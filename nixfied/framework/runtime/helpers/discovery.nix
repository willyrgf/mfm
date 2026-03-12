# Repository discovery generator and validator.
{
  pkgs,
  project,
  loggingPrelude ? "",
  commandSurfaces ? null,
  featureInventory ? null,
}:

let
  cfg = project.discovery or { };
  enabled = cfg.enable or true;
  strict = cfg.strict or true;
  refreshArg = cfg.refreshArg or "--refresh-discovery";
  resolvedCommandSurfaces = cfg.commandSurfaces or commandSurfaces;
  resolvedFeatureInventory = cfg.featureInventory or featureInventory;

  defaultRequiredDocs = [
    "README.md"
    "AGENTS.md"
    "docs/ARCHITECTURE.md"
    "docs/DETAILED.md"
    "docs/UPGRADE.md"
  ];
  requiredDocs = cfg.requiredDocs or defaultRequiredDocs;

  defaultRiskAreas = [
    {
      path = "nixfied/project/conf.nix";
      risk = "Project identity, environment names, and port contract.";
      required_checks = [
        "nix run .#validate-env"
        "nix run .#ci -- --summary"
      ];
    }
    {
      path = "nixfied/project/module.nix";
      risk = "Primary command/task/workflow surface.";
      required_checks = [
        "nix run .#help"
        "nix run .#framework::test"
        "nix run .#ci -- --summary"
      ];
    }
    {
      path = "nixfied/framework";
      risk = "Framework-owned presets, runtime, and install internals; avoid direct edits in installed repos.";
      required_checks = [
        "nix run .#help"
      ];
    }
  ];
  riskAreas = cfg.riskAreas or defaultRiskAreas;

  requiredDocsJson = builtins.toJSON requiredDocs;
  riskAreasJson = builtins.toJSON riskAreas;
  commandSurfacesJson =
    if resolvedCommandSurfaces == null then "null" else builtins.toJSON resolvedCommandSurfaces;
  featureInventoryJson =
    if resolvedFeatureInventory == null then "null" else builtins.toJSON resolvedFeatureInventory;

  tool = pkgs.writeShellScriptBin "nixfied-discovery-index" ''
        ${loggingPrelude}

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
                log_error "--root requires a path"
                exit 1
              fi
              shift 2
              ;;
            --help|-h)
              usage
              exit 0
              ;;
            *)
              log_error "unknown option: $1"
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
        COMPILED_COMMAND_SURFACES_JSON='${commandSurfacesJson}'
        COMPILED_FEATURE_INVENTORY_JSON='${featureInventoryJson}'
        HAS_GIT_TRACKING="0"
        if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
          HAS_GIT_TRACKING="1"
        fi

        doc_purpose() {
          case "$1" in
            README.md) echo "Primary repository overview and command entrypoints." ;;
            AGENTS.md) echo "Agent instructions and collaboration constraints." ;;
            CLAUDE.md) echo "Additional assistant guidance for this repository." ;;
            ARCHITECTURE.md|docs/ARCHITECTURE.md) echo "High-level architecture reference." ;;
            docs/DETAILED.md) echo "Detailed model architecture and contracts." ;;
            docs/UPGRADE.md) echo "Downstream upgrade notes for behavioral and path contract changes." ;;
            REDESIGN.md) echo "Redesign notes and migration context." ;;
            *) echo "Project documentation." ;;
          esac
        }

        doc_priority() {
          case "$1" in
            README.md) echo 1 ;;
            docs/DETAILED.md) echo 2 ;;
            docs/UPGRADE.md) echo 3 ;;
            ARCHITECTURE.md|docs/ARCHITECTURE.md) echo 4 ;;
            REDESIGN.md) echo 6 ;;
            AGENTS.md) echo 7 ;;
            CLAUDE.md) echo 7 ;;
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

        if [ "$COMPILED_COMMAND_SURFACES_JSON" = "null" ]; then
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
        else
          COMMANDS_JSON="$(
            printf '%s\n' "$COMPILED_COMMAND_SURFACES_JSON" | ${pkgs.jq}/bin/jq -c '
              map(
                select((.name // "") != "")
                | {
                    name: .name,
                    owner_file:
                      ((.owner_file // .ownerFile // "nixfied/project/module.nix")
                       | if . == "" then "nixfied/project/module.nix" else . end)
                  }
              )
              | unique_by(.name + "\u0000" + .owner_file)
              | sort_by(.name, .owner_file)
            '
          )"
        fi

        if [ "$COMPILED_FEATURE_INVENTORY_JSON" = "null" ]; then
          FEATURES_JSON='[]'
        else
          FEATURES_JSON="$(
            printf '%s\n' "$COMPILED_FEATURE_INVENTORY_JSON" | ${pkgs.jq}/bin/jq -c '
              if type == "array" then
                .
              else
                to_entries | map(.value + { id: (.value.id // .key) })
              end
              | map(
                  select((.id // "") != "")
                  | {
                      id: .id,
                      kind: (.kind // "unknown"),
                      summary: (.summary // ""),
                      status: (.status // ""),
                      coverage_required: (.coverageRequired // false)
                    }
                )
              | unique_by(.id)
              | sort_by(.id)
            '
          )"
        fi

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
          --argjson features "$FEATURES_JSON" \
          --argjson risk_areas "$RISK_AREAS_NORM_JSON" \
          '{
            schema_version: 1,
            generated_by: "nixfied-discovery-index",
            docs: $docs,
            components: $components,
            risk_areas: $risk_areas,
            command_surfaces: $command_surfaces,
            features: $features
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
          echo "## Features"
          ${pkgs.jq}/bin/jq -r '
            if (.features | length) == 0 then
              "- (none detected)"
            else
              .features[]
              | "- `" + .id + "` [" + .kind + "] - " + .summary
                + (if .coverage_required then " (coverage required)" else "" end)
            end
          ' "$TMP_INDEX"
          echo ""
          echo "## Dispatcher and Introspection"
          echo '- `run-task -- <task-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `run-workflow -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `run-workflow-parallel -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `runs [run-id]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `stop-run -- <run-id>` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `stop-all-runs` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `features` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `model`, `stateHash`, `tasks`, `services`, `task::<id>`, `schema` from `nixfied/framework/core/mkNixfied.nix`'
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
          echo '- `nix run .#format`'
          echo '- `nix run .#ci -- --summary`'
          echo '- `nix run .#validate-env`'
          echo '- `nix run .#test-isolation`'
          echo '- `nix run .#ports`'
          echo '- `nix run .#check-ports`'
          echo '- `nix run .#features`'
          echo '- `nix run .#framework::test`'
          echo '- `nix run .#framework::install`'
          echo '- `nix run .#framework::upgrade`'
          echo ""
          echo "## Invariants"
          echo '- Treat `nixfied/project/` as the primary customization surface.'
          echo '- Keep command metadata aligned with script behavior.'
          echo '- Keep this map and `docs/repo-index.json` in sync when command surfaces or key docs change.'
        } > "$TMP_MAP"

        verify_file() {
          local generated_path="$1"
          local committed_path="$2"

          if [ ! -f "$committed_path" ]; then
            log_error "missing discovery artifact path=''${committed_path#"$ROOT"/}"
            return 1
          fi

          if ! ${pkgs.diffutils}/bin/cmp -s "$generated_path" "$committed_path"; then
            log_error "discovery artifact drift path=''${committed_path#"$ROOT"/}"
            ${pkgs.diffutils}/bin/diff -u "$committed_path" "$generated_path" >&2 || true
            return 1
          fi

          log_ok "discovery artifact current path=''${committed_path#"$ROOT"/}"
          return 0
        }

        write_if_changed() {
          local generated_path="$1"
          local committed_path="$2"

          if [ -f "$committed_path" ] && ${pkgs.diffutils}/bin/cmp -s "$generated_path" "$committed_path"; then
            log_ok "discovery artifact unchanged path=''${committed_path#"$ROOT"/}"
            return 0
          fi

          ${pkgs.coreutils}/bin/mkdir -p "$(${pkgs.coreutils}/bin/dirname "$committed_path")"
          ${pkgs.coreutils}/bin/cp "$generated_path" "$committed_path"
          log_info "wrote discovery artifact path=''${committed_path#"$ROOT"/}"
          return 0
        }

        if [ "$MODE" = "refresh" ]; then
          write_if_changed "$TMP_INDEX" "$INDEX_PATH"
          write_if_changed "$TMP_MAP" "$MAP_PATH"
          log_ok "discovery refresh complete root=$ROOT"
          exit 0
        fi

        rc=0
        verify_file "$TMP_INDEX" "$INDEX_PATH" || rc=1
        verify_file "$TMP_MAP" "$MAP_PATH" || rc=1

        if [ "$rc" -ne 0 ]; then
          log_error "discovery artifacts are out of date."
          log_info "refresh the committed discovery artifacts with the configured discovery generator"
          exit "$rc"
        fi

        log_ok "discovery artifacts are current"
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
