{
  pkgs,
  lib,
  frameworkRoot,
}:

pkgs.writeShellScript "nixfied-prompt-plan" ''
    ${lib.loggingPrelude}

    set -euo pipefail

    FORCE=false
    OUT_PATH=""

    require_next_arg() {
      nixfied_require_next_arg "$@"
    }

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --force)
          FORCE=true
          shift
          ;;
        --output=*)
          OUT_PATH="''${1#--output=}"
          shift
          ;;
        --output)
          OUT_PATH="$(require_next_arg --output "a path" "$@")"
          shift 2
          ;;
        --help|-h)
          echo "Usage: nix run github:willyrgf/nixfied#framework::prompt-plan [--force] [--output=PATH]"
          exit 0
          ;;
        --)
          shift
          ;;
        *)
          shift
          ;;
      esac
    done

    if [ "''${NIXFIED_PROMPT_PLAN:-1}" = "0" ] || [ "''${NIXFIED_PROMPT_PLAN:-}" = "false" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "0" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "false" ]; then
      log_info "Prompt plan disabled (NIXFIED_PROMPT_PLAN=0)."
      exit 0
    fi

    ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
    if [ -z "$ROOT" ]; then
      nixfied_exit_precondition "Not inside a git repository."
    fi

    OUT_PATH="''${OUT_PATH:-$ROOT/NIXFIED_PROMPT_PLAN.md}"

    if [ -f "$OUT_PATH" ] && [ "$FORCE" = "false" ] && [ "''${NIXFIED_PROMPT_PLAN_OVERWRITE:-0}" != "1" ] && [ "''${NIXFIED_INTEGRATION_PLAN_OVERWRITE:-0}" != "1" ]; then
      log_info "Prompt plan already exists: $OUT_PATH"
      echo "    Re-run with --force or NIXFIED_PROMPT_PLAN_OVERWRITE=1 to overwrite."
      exit 0
    fi

    if ! command -v nix >/dev/null 2>&1; then
      nixfied_exit_unavailable "nix is required to run dump2llm."
    fi

    CONTEXT_FILE=$(mktemp)
    PROMPT_FILE=$(mktemp)
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR" "$CONTEXT_FILE" "$PROMPT_FILE"' EXIT

    add_prompt_input() {
      local src="$1"
      local dest="$2"
      if [ -f "$src" ]; then
        cp "$src" "$TMPDIR/$dest"
        INPUTS+=("$dest")
      fi
    }

    INPUTS=()

    add_prompt_input "$ROOT/README.md" "PROJECT_README.md"
    add_prompt_input "$ROOT/CLAUDE.md" "PROJECT_CLAUDE.md"
    add_prompt_input "$ROOT/AGENTS.md" "PROJECT_AGENTS.md"

    FRAMEWORK_README="${frameworkRoot}/README.md"
    add_prompt_input "$FRAMEWORK_README" "NIXFIED_FRAMEWORK_README.md"

    if [ "''${#INPUTS[@]}" -eq 0 ]; then
      log_skip "Skipping prompt plan (no README/CLAUDE/AGENTS files found)."
      exit 0
    fi

    cat > "$PROMPT_FILE" <<'EOF'
  Create a PROMPT PLAN in Markdown for integrating this project with the Nixfied framework.

  Requirements:
  - Be concise and actionable.
  - Use headings: "PROMPT PLAN", "Project Snapshot", "Current Behavior", "Integration Steps",
    "Key Files to Edit", "Validation Checklist", "Open Questions", and "Next Prompts".
  - Ground every step in the provided context; do not guess missing details.
  - Treat Nixfied as the single entrypoint for dev/test/build/check/ci and (optionally) db/nginx/supervisor:
    nix run .#help, .#dev, .#test, .#build, .#check, .#ci
  - Call out the key file-to-command mapping:
    - nixfied/project/module.nix + nixfied/project/{tasks,workflows}.nix + nixfied/framework/presets/*.nix -> command/task/workflow composition
    - nixfied/project/conf.nix -> project identity, envs/ports, module toggles, runtime defaults
  - Mention the primary customization surface is nixfied/project/ (avoid editing flake.nix unless the plan proves it's necessary).
  - Reference relevant framework features (only if applicable to this project):
    - CI pipeline DSL (sequential steps or staged parallel groups with maxWorkers/locks, artifacts, summary.json; supports --summary and --mode/--<mode>)
    - Ephemeral environments (slot locking, source copy, conditional cleanup; ci.useEphemeral)
    - Module apps + hooks (db-*, nginx-*, supervisor apps; postgres backups/migrations)
    - Run registry (event log and workflow/task state tracking)
  - In "Integration Steps", start with high-level goals (behavior parity with the current dev/test/build/check/ci workflows, avoid regressions), then list concrete wiring steps with exact file paths.
  - In "Key Files to Edit", list each file and the specific changes needed.
  - In "Validation Checklist", include concrete smoke checks (nix run .#help/.#dev/.#test/.#build/.#check/.#ci -- --summary) and any project-specific checks from the docs.
  - Include documentation alignment goals (README.md plus any agent instruction docs like CLAUDE.md/AGENTS.md should make Nixfied the canonical entrypoint).
  - If docs conflict on command names or behavior, call it out and ask which source is authoritative.
  - If info is missing, list it in "Open Questions".

  Sources:
  - Project docs: PROJECT_README.md, PROJECT_CLAUDE.md, PROJECT_AGENTS.md
  - Nixfied docs: NIXFIED_FRAMEWORK_README.md

  Context (project docs + framework README) follows:
  EOF

    CONTEXT_STATUS=0
    if ! (cd "$TMPDIR" && nix run github:willyrgf/dump2llm -- "''${INPUTS[@]}") > "$CONTEXT_FILE"; then
      CONTEXT_STATUS=1
    fi

    {
      echo "# NIXFIED PROMPT PLAN"
      echo ""
      cat "$PROMPT_FILE"
      echo ""
      if [ "$CONTEXT_STATUS" -eq 0 ]; then
        cat "$CONTEXT_FILE"
      else
        echo ""
        log_warn "Context generation failed. Re-run the prompt plan:"
        echo ""
        echo "  nix run github:willyrgf/nixfied#framework::prompt-plan -- --force"
      fi
    } > "$OUT_PATH"

    log_ok "Prompt plan written to $OUT_PATH"
''
