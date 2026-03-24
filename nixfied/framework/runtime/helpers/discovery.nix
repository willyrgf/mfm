# Repository discovery generator and validator.
{
  pkgs,
  project,
  loggingPrelude ? "",
  commandSurfaces ? null,
  featureInventory ? null,
}:

let
  lib = pkgs.lib;
  cfg = project.discovery or { };
  enabled = cfg.enable or true;
  strict = cfg.strict or true;
  refreshArg = cfg.refreshArg or "--refresh-discovery";
  resolvedCommandSurfaces = cfg.commandSurfaces or commandSurfaces;
  resolvedFeatureInventory = cfg.featureInventory or featureInventory;
  commonRuntimeShell = import ../common-runtime.nix { inherit pkgs; };

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

  requiredDocsLines = builtins.concatStringsSep "\n" requiredDocs;
  compiledCommandSurfaceLines =
    if resolvedCommandSurfaces == null then
      ""
    else
      builtins.concatStringsSep "\n" (
        builtins.sort builtins.lessThan (
          lib.unique (
            map (
              entry:
              let
                name = entry.name or "";
                ownerFile =
                  let
                    rawOwner = entry.owner_file or entry.ownerFile or "nixfied/project/module.nix";
                  in
                  if rawOwner == "" then "nixfied/project/module.nix" else rawOwner;
              in
              if name == "" then "" else "${name}\t${ownerFile}"
            ) resolvedCommandSurfaces
          )
        )
      );
  compiledFeatureLines =
    if resolvedFeatureInventory == null then
      ""
    else
      let
        rawFeatures =
          if builtins.isList resolvedFeatureInventory then
            resolvedFeatureInventory
          else
            map (
              key:
              (resolvedFeatureInventory.${key} or { })
              // {
                id = (resolvedFeatureInventory.${key}.id or key);
              }
            ) (builtins.attrNames resolvedFeatureInventory);
        normalized = builtins.foldl' (
          acc: feature:
          let
            featureId = feature.id or "";
          in
          if featureId == "" then
            acc
          else
            acc
            // {
              ${featureId} = {
                id = featureId;
                kind = feature.kind or "unknown";
                summary = feature.summary or "";
                status = feature.status or "";
                coverageRequired = feature.coverageRequired or false;
              };
            }
        ) { } rawFeatures;
      in
      builtins.concatStringsSep "\n" (
        map (
          featureId:
          let
            feature = normalized.${featureId};
          in
          "${feature.id}\t${feature.kind}\t${feature.summary}\t${feature.status}\t${
            if feature.coverageRequired then "true" else "false"
          }"
        ) (builtins.sort builtins.lessThan (builtins.attrNames normalized))
      );
  riskAreaLines = builtins.concatStringsSep "\n" (
    builtins.sort builtins.lessThan (
      map (
        area: "${area.path}\t${area.risk}\t${builtins.concatStringsSep "," (area.required_checks or [ ])}"
      ) riskAreas
    )
  );

  tool = pkgs.writeShellScriptBin "nixfied-discovery-index" ''
        ${loggingPrelude}
        ${commonRuntimeShell}

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

        REQUIRED_DOCS_LINES=${lib.escapeShellArg requiredDocsLines}
        RISK_AREA_LINES=${lib.escapeShellArg riskAreaLines}
        COMPILED_COMMAND_SURFACE_LINES=${lib.escapeShellArg compiledCommandSurfaceLines}
        COMPILED_FEATURE_LINES=${lib.escapeShellArg compiledFeatureLines}
        HAS_GIT_TRACKING="0"
        if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
          HAS_GIT_TRACKING="1"
        fi

        render_docs_json() {
          local docs_tsv="$1"
          local path=""
          local purpose=""
          local priority=""
          local exists=""
          local exists_json="false"
          local first=1

          printf '['
          while IFS=$'\t' read -r path purpose priority exists; do
            [ -n "$path" ] || continue
            if [ "$exists" = "true" ]; then
              exists_json="true"
            else
              exists_json="false"
            fi
            if [ "$first" -eq 0 ]; then
              printf ','
            fi
            printf '{'
            printf '"path":%s' "$(json_quote_string "$path")"
            printf ',"purpose":%s' "$(json_quote_string "$purpose")"
            printf ',"priority":%s' "$priority"
            printf ',"exists":%s' "$exists_json"
            printf '}'
            first=0
          done < "$docs_tsv"
          printf ']'
        }

        render_components_json() {
          local components_tsv="$1"
          local name=""
          local path=""
          local kind=""
          local first=1

          printf '['
          while IFS=$'\t' read -r name path kind; do
            [ -n "$path" ] || continue
            if [ "$first" -eq 0 ]; then
              printf ','
            fi
            printf '{'
            printf '"name":%s' "$(json_quote_string "$name")"
            printf ',"path":%s' "$(json_quote_string "$path")"
            printf ',"kind":%s' "$(json_quote_string "$kind")"
            printf '}'
            first=0
          done < "$components_tsv"
          printf ']'
        }

        render_command_surfaces_json() {
          local commands_tsv="$1"
          local name=""
          local owner_file=""
          local first=1

          printf '['
          while IFS=$'\t' read -r name owner_file; do
            [ -n "$name" ] || continue
            if [ "$first" -eq 0 ]; then
              printf ','
            fi
            printf '{'
            printf '"name":%s' "$(json_quote_string "$name")"
            printf ',"owner_file":%s' "$(json_quote_string "$owner_file")"
            printf '}'
            first=0
          done < "$commands_tsv"
          printf ']'
        }

        render_features_json() {
          local features_tsv="$1"
          local id=""
          local kind=""
          local summary=""
          local status=""
          local coverage_required=""
          local coverage_json="false"
          local first=1

          printf '['
          while IFS=$'\t' read -r id kind summary status coverage_required; do
            [ -n "$id" ] || continue
            if [ "$coverage_required" = "true" ]; then
              coverage_json="true"
            else
              coverage_json="false"
            fi
            if [ "$first" -eq 0 ]; then
              printf ','
            fi
            printf '{'
            printf '"id":%s' "$(json_quote_string "$id")"
            printf ',"kind":%s' "$(json_quote_string "$kind")"
            printf ',"summary":%s' "$(json_quote_string "$summary")"
            printf ',"status":%s' "$(json_quote_string "$status")"
            printf ',"coverage_required":%s' "$coverage_json"
            printf '}'
            first=0
          done < "$features_tsv"
          printf ']'
        }

        render_risk_areas_json() {
          local risk_tsv="$1"
          local path=""
          local risk=""
          local checks_csv=""
          local first=1
          local check=""
          local checks_json=""
          local checks_first=1

          printf '['
          while IFS=$'\t' read -r path risk checks_csv; do
            [ -n "$path" ] || continue
            checks_json="["
            checks_first=1
            if [ -n "$checks_csv" ]; then
              while IFS= read -r check; do
                [ -n "$check" ] || continue
                if [ "$checks_first" -eq 0 ]; then
                  checks_json="$checks_json,"
                fi
                checks_json="$checks_json$(json_quote_string "$check")"
                checks_first=0
              done < <(printf '%s' "$checks_csv" | ${pkgs.gnused}/bin/sed 's/,/\n/g')
            fi
            checks_json="$checks_json]"
            if [ "$first" -eq 0 ]; then
              printf ','
            fi
            printf '{'
            printf '"path":%s' "$(json_quote_string "$path")"
            printf ',"risk":%s' "$(json_quote_string "$risk")"
            printf ',"required_checks":%s' "$checks_json"
            printf '}'
            first=0
          done < "$risk_tsv"
          printf ']'
        }

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
        done < <(printf '%s\n' "$REQUIRED_DOCS_LINES")

        DOCS_SORTED_TSV="$TMP_DIR/docs.sorted.tsv"
        ${pkgs.coreutils}/bin/sort -t $'\t' -k3,3n -k1,1 "$DOCS_TSV" > "$DOCS_SORTED_TSV"
        DOCS_JSON="$(render_docs_json "$DOCS_SORTED_TSV")"

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

        COMPONENTS_SORTED_TSV="$TMP_DIR/components.sorted.tsv"
        ${pkgs.coreutils}/bin/sort -t $'\t' -k2,2 -u "$COMPONENTS_TSV" > "$COMPONENTS_SORTED_TSV"
        COMPONENTS_JSON="$(render_components_json "$COMPONENTS_SORTED_TSV")"

        COMMANDS_TSV="$TMP_DIR/commands.tsv"
        : > "$COMMANDS_TSV"
        if [ -n "$COMPILED_COMMAND_SURFACE_LINES" ]; then
          printf '%s\n' "$COMPILED_COMMAND_SURFACE_LINES" > "$COMMANDS_TSV"
        elif [ -d "$ROOT/nixfied/project" ]; then
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
        COMMANDS_SORTED_TSV="$TMP_DIR/commands.sorted.tsv"
        ${pkgs.coreutils}/bin/sort -t $'\t' -k1,1 -k2,2 -u "$COMMANDS_TSV" > "$COMMANDS_SORTED_TSV"
        COMMANDS_JSON="$(render_command_surfaces_json "$COMMANDS_SORTED_TSV")"

        FEATURES_TSV="$TMP_DIR/features.tsv"
        : > "$FEATURES_TSV"
        if [ -n "$COMPILED_FEATURE_LINES" ]; then
          printf '%s\n' "$COMPILED_FEATURE_LINES" > "$FEATURES_TSV"
        fi
        FEATURES_JSON="$(render_features_json "$FEATURES_TSV")"

        RISK_TSV="$TMP_DIR/risk-areas.tsv"
        : > "$RISK_TSV"
        if [ -n "$RISK_AREA_LINES" ]; then
          printf '%s\n' "$RISK_AREA_LINES" > "$RISK_TSV"
        fi
        RISK_AREAS_JSON="$(render_risk_areas_json "$RISK_TSV")"

        {
          printf '{'
          printf '"schema_version":1'
          printf ',"generated_by":"nixfied-discovery-index"'
          printf ',"docs":%s' "$DOCS_JSON"
          printf ',"components":%s' "$COMPONENTS_JSON"
          printf ',"risk_areas":%s' "$RISK_AREAS_JSON"
          printf ',"command_surfaces":%s' "$COMMANDS_JSON"
          printf ',"features":%s' "$FEATURES_JSON"
          printf '}\n'
        } > "$TMP_INDEX"

        {
          echo "# Repository Map"
          echo ""
          echo "Generated from \`docs/repo-index.json\`."
          echo ""
          echo "## Start Here"
          while IFS=$'\t' read -r doc_path doc_purpose doc_priority doc_exists; do
            [ -n "$doc_path" ] || continue
            if [ "$doc_exists" = "true" ]; then
              echo "- \`$doc_path\` - $doc_purpose"
            else
              echo "- \`$doc_path\` - $doc_purpose (missing)"
            fi
          done < "$DOCS_SORTED_TSV"
          echo ""
          echo "## Components"
          if [ -s "$COMPONENTS_SORTED_TSV" ]; then
            while IFS=$'\t' read -r component_name component_path component_kind; do
              [ -n "$component_path" ] || continue
              echo "- \`$component_path\` ($component_kind)"
            done < "$COMPONENTS_SORTED_TSV"
          else
            echo "- (none detected)"
          fi
          echo ""
          echo "## Command Surfaces"
          if [ -s "$COMMANDS_SORTED_TSV" ]; then
            while IFS=$'\t' read -r command_name owner_file; do
              [ -n "$command_name" ] || continue
              echo "- \`$command_name\` from \`$owner_file\`"
            done < "$COMMANDS_SORTED_TSV"
          else
            echo "- (none detected)"
          fi
          echo ""
          echo "## Features"
          if [ -s "$FEATURES_TSV" ]; then
            while IFS=$'\t' read -r feature_id feature_kind feature_summary feature_status feature_coverage_required; do
              [ -n "$feature_id" ] || continue
              if [ "$feature_coverage_required" = "true" ]; then
                echo "- \`$feature_id\` [$feature_kind] - $feature_summary (coverage required)"
              else
                echo "- \`$feature_id\` [$feature_kind] - $feature_summary"
              fi
            done < "$FEATURES_TSV"
          else
            echo "- (none detected)"
          fi
          echo ""
          echo "## Dispatcher and Introspection"
          echo '- `run-task -- <task-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `run-workflow -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `run-workflow-parallel -- <workflow-id> [-- ...]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `runs [run-id]` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `stop-run -- <run-id>` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `stop-all-runs` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `features` from `nixfied/framework/runtime/dispatcher.nix`'
          echo '- `introspect`, `stateHash`, `schema` from `nixfied/framework/core/mkNixfied.nix`'
          echo ""
          echo "## Sensitive Zones"
          if [ -s "$RISK_TSV" ]; then
            while IFS=$'\t' read -r risk_path risk_text risk_checks; do
              [ -n "$risk_path" ] || continue
              if [ -n "$risk_checks" ]; then
                echo "- \`$risk_path\` - $risk_text (checks: $risk_checks)"
              else
                echo "- \`$risk_path\` - $risk_text"
              fi
            done < "$RISK_TSV"
          else
            echo "- (none configured)"
          fi
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
