# Installer app for the framework
{
  pkgs,
  lib,
  frameworkRoot,
}:

let
  inherit (lib) mkApp;

  promptPlanScript = pkgs.writeShellScript "nixfied-prompt-plan" ''
            set -euo pipefail

            FORCE=false
            OUT_PATH=""

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
                  if [ "$#" -lt 2 ]; then
                    echo "❌ --output requires a path" >&2
                    exit 1
                  fi
                  OUT_PATH="''${2-}"
                  shift 2
                  ;;
                --help|-h)
                  echo "Usage: nix run github:willyrgf/nixfied#framework::prompt-plan [--force] [--output=PATH]"
                  exit 0
                  ;;
                --)
                  # Accept an explicit "--" (some wrappers include it) and keep parsing.
                  shift
                  ;;
                *)
                  # Ignore unknown args for forward compatibility.
                  shift
                  ;;
              esac
            done

            if [ "''${NIXFIED_PROMPT_PLAN:-1}" = "0" ] || [ "''${NIXFIED_PROMPT_PLAN:-}" = "false" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "0" ] || [ "''${NIXFIED_INTEGRATION_PLAN:-}" = "false" ]; then
              echo "ℹ️  Prompt plan disabled (NIXFIED_PROMPT_PLAN=0)."
              exit 0
            fi

            ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
            if [ -z "$ROOT" ]; then
              echo "❌ Not inside a git repository." >&2
              exit 1
            fi

            OUT_PATH="''${OUT_PATH:-$ROOT/NIXFIED_PROMPT_PLAN.md}"

            if [ -f "$OUT_PATH" ] && [ "$FORCE" = "false" ] && [ "''${NIXFIED_PROMPT_PLAN_OVERWRITE:-0}" != "1" ] && [ "''${NIXFIED_INTEGRATION_PLAN_OVERWRITE:-0}" != "1" ]; then
              echo "ℹ️  Prompt plan already exists: $OUT_PATH"
              echo "    Re-run with --force or NIXFIED_PROMPT_PLAN_OVERWRITE=1 to overwrite."
              exit 0
            fi

            if ! command -v nix >/dev/null 2>&1; then
              echo "❌ nix is required to run dump2llm." >&2
              exit 1
            fi

        CONTEXT_FILE=$(mktemp)
        PROMPT_FILE=$(mktemp)
        TMPDIR=$(mktemp -d)
        trap 'rm -rf "$TMPDIR" "$CONTEXT_FILE" "$PROMPT_FILE"' EXIT

        INPUTS=()

        if [ -f "$ROOT/README.md" ]; then
          cp "$ROOT/README.md" "$TMPDIR/PROJECT_README.md"
          INPUTS+=("PROJECT_README.md")
        fi
        if [ -f "$ROOT/CLAUDE.md" ]; then
          cp "$ROOT/CLAUDE.md" "$TMPDIR/PROJECT_CLAUDE.md"
          INPUTS+=("PROJECT_CLAUDE.md")
        fi
        if [ -f "$ROOT/AGENTS.md" ]; then
          cp "$ROOT/AGENTS.md" "$TMPDIR/PROJECT_AGENTS.md"
          INPUTS+=("PROJECT_AGENTS.md")
        fi

        FRAMEWORK_README="${frameworkRoot}/README.md"
        if [ -f "$FRAMEWORK_README" ]; then
          cp "$FRAMEWORK_README" "$TMPDIR/NIXFIED_FRAMEWORK_README.md"
          INPUTS+=("NIXFIED_FRAMEWORK_README.md")
        fi

        if [ "''${#INPUTS[@]}" -eq 0 ]; then
          echo "ℹ️  Skipping prompt plan (no README/CLAUDE/AGENTS files found)." >&2
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
    - Call out the key file-to-command mapping (do not assume "prod" is a command):
      - nixfied/project/dev.nix -> commands.dev
      - nixfied/project/test.nix -> commands.test
      - nixfied/project/prod.nix -> commands.build (build/prod workflow)
      - nixfied/project/quality.nix -> commands.check
      - nixfied/project/ci.nix -> CI pipeline DSL config (ci.modes/ci.steps) + CI command metadata
      - nixfied/project/conf.nix -> project identity, envs/ports, module toggles, ephemeral config
      - nixfied/project/default.nix -> merges all project files; update if new files are added
    - Mention the primary customization surface is nixfied/project/ (avoid editing flake.nix unless the plan proves it's necessary).
    - Reference relevant framework features (only if applicable to this project):
      - CI pipeline DSL (modes/steps, artifacts, summary.json; supports --summary, --mode/--<mode>, --bg)
      - Ephemeral environments (slot locking, source copy, conditional cleanup; ci.useEphemeral)
      - Module apps + hooks (db-*, nginx-*, supervisor apps; postgres backups/migrations)
      - Run registry (used by CI --bg mode)
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
            echo "⚠️  Context generation failed. Re-run the prompt plan:"
            echo ""
            echo "  nix run github:willyrgf/nixfied#framework::prompt-plan -- --force"
          fi
        } > "$OUT_PATH"

            echo "📝 Prompt plan written to $OUT_PATH"
  '';

  installScript = ''
    set -euo pipefail

    ORIG_ARGS=("$@")
    FORCE=false
    FILTERS_RAW=""
    TARGET_PATH=""
    USE_WORKTREE=false
    SYNC_TARGET=false
    PROMPT_PLAN=false
    PROMPT_PLAN_FORCE=false
    RESET_PROJECT=false
    MODE="''${NIXFIED_INSTALL_MODE:-install}"

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --force)
          FORCE=true
          shift
          ;;
        --upgrade)
          MODE="upgrade"
          shift
          ;;
        --filter=*)
          FILTERS_RAW="''${1#--filter=}"
          shift
          ;;
        --filter)
          if [ "$#" -lt 2 ]; then
            echo "❌ --filter requires a value (example: --filter=conf,ci)" >&2
            exit 1
          fi
          FILTERS_RAW="''${2-}"
          shift 2
          ;;
        --target=*)
          TARGET_PATH="''${1#--target=}"
          shift
          ;;
        --target)
          if [ "$#" -lt 2 ]; then
            echo "❌ --target requires a path" >&2
            exit 1
          fi
          TARGET_PATH="''${2-}"
          shift 2
          ;;
        --worktree)
          USE_WORKTREE=true
          shift
          ;;
        --sync)
          SYNC_TARGET=true
          shift
          ;;
        --prompt-plan)
          PROMPT_PLAN=true
          shift
          ;;
        --prompt-plan-force)
          PROMPT_PLAN=true
          PROMPT_PLAN_FORCE=true
          shift
          ;;
        --reset-project)
          RESET_PROJECT=true
          shift
          ;;
        --help|-h)
          cat <<'EOF'
Usage:
  nix run github:willyrgf/nixfied#framework::install [options]
  nix run github:willyrgf/nixfied#framework::upgrade [options]

Install options:
  --force                Overwrite existing nix files (flake.nix/flake.lock/nixfied)
  --filter=LIST          Fresh install only: install subset of project templates
                         (values: conf,dev,test,build,quality,ci; build is an alias of prod.nix)
  --target=PATH          When installing from a non-_nixified repo, write to PATH (must end with _nixified)
  --worktree             Use 'git worktree add --detach' instead of copying (requires clean working tree unless --force)
  --sync                 If target exists and --force is set, sync repo contents into target before installing

Upgrade options:
  --upgrade              Treat as an upgrade (preserves nixfied/project unless --reset-project)
  --reset-project        Overwrite nixfied/project templates during upgrade

Prompt plan:
  --prompt-plan          Generate NIXFIED_PROMPT_PLAN.md after install/upgrade (best effort)
  --prompt-plan-force    Overwrite existing NIXFIED_PROMPT_PLAN.md
EOF
          exit 0
          ;;
        --)
          # Accept an explicit "--" (some wrappers include it) and keep parsing.
          shift
          ;;
        *)
          # Ignore unknown args for forward compatibility.
          shift
          ;;
      esac
    done

    if [ "$FORCE" = "true" ]; then
      export NIXFIED_INSTALL_FORCE=1
    fi

    ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
    if [ -z "$ROOT" ]; then
      echo "❌ Not inside a git repository." >&2
      exit 1
    fi

    SUFFIX="_nixified"
    BASE=$(basename "$ROOT")

    if [ "$MODE" = "upgrade" ]; then
      if [[ "$BASE" != *"$SUFFIX" ]]; then
        echo "❌ Upgrade requires running inside a repository ending with $SUFFIX" >&2
        exit 1
      fi
      if [ ! -d "$ROOT/nixfied" ]; then
        echo "❌ No nixfied/ directory found in $ROOT" >&2
        echo "   Run install first: nix run github:willyrgf/nixfied#framework::install" >&2
        exit 1
      fi
    fi

    if [ "$MODE" != "upgrade" ] && [ -z "''${NIXFIED_INSTALL_REENTRY:-}" ]; then
      if [[ "$BASE" != *"$SUFFIX" ]]; then
        TARGET="''${TARGET_PATH:-''${ROOT}''${SUFFIX}}"
        if [[ "$(basename "$TARGET")" != *"$SUFFIX" ]]; then
          echo "❌ Target must end with $SUFFIX (got: $TARGET)" >&2
          exit 1
        fi
        case "$TARGET" in
          "$ROOT"/*)
            echo "❌ Target must not be inside the source repo (got: $TARGET)" >&2
            exit 1
            ;;
        esac
        if [ -e "$TARGET" ]; then
          if [ "$FORCE" = "true" ]; then
            if [ "$SYNC_TARGET" = "true" ] && [ "$USE_WORKTREE" != "true" ]; then
              echo "🔄 Syncing repository to $TARGET..."
              ${pkgs.rsync}/bin/rsync -a --delete "$ROOT/" "$TARGET/"
            else
              echo "⚠️  Target already exists: $TARGET"
              echo "    Reusing existing target (no new copy/worktree created)."
              if [ "$SYNC_TARGET" = "true" ] && [ "$USE_WORKTREE" = "true" ]; then
                echo "ℹ️  --sync has no effect with --worktree."
              fi
            fi
            (cd "$TARGET" && NIXFIED_INSTALL_REENTRY=1 NIXFIED_INSTALL_FORCE=1 "$0" "''${ORIG_ARGS[@]}")
            exit 0
          else
            echo "❌ Target already exists: $TARGET" >&2
            echo "   Remove it or rename it, then re-run (or pass --force to reuse)." >&2
            exit 1
          fi
        fi
        if [ "$USE_WORKTREE" = "true" ]; then
          if [ ! -x "${pkgs.git}/bin/git" ]; then
            echo "❌ git is required for --worktree" >&2
            exit 1
          fi
          DIRTY=$(${pkgs.git}/bin/git -C "$ROOT" status --porcelain 2>/dev/null || true)
          if [ -n "$DIRTY" ] && [ "$FORCE" != "true" ]; then
            echo "❌ Working tree is dirty; refusing to create a detached worktree without --force" >&2
            echo "   (uncommitted changes would not be present in the worktree)" >&2
            exit 1
          fi
          echo "🌿 Creating git worktree at $TARGET..."
          ${pkgs.git}/bin/git -C "$ROOT" worktree add --detach "$TARGET" >/dev/null
        else
          echo "📦 Copying repository to $TARGET..."
          ${pkgs.rsync}/bin/rsync -a "$ROOT/" "$TARGET/"
        fi
        echo "✅ Target ready. Re-running installer in $TARGET"
        (cd "$TARGET" && NIXFIED_INSTALL_REENTRY=1 "$0" "''${ORIG_ARGS[@]}")
        exit 0
      fi
    fi

    if [[ "$BASE" != *"$SUFFIX" ]]; then
      echo "❌ For safety, run inside a repository ending with $SUFFIX" >&2
      exit 1
    fi

    SRC="${frameworkRoot}"

    if [ ! -f "$SRC/flake.nix" ] || [ ! -d "$SRC/nixfied" ]; then
      echo "❌ Framework source is missing required files." >&2
      exit 1
    fi

    NEEDS_OVERWRITE=false
    if [ -e "$ROOT/flake.nix" ] || [ -e "$ROOT/flake.lock" ] || [ -d "$ROOT/nixfied" ]; then
      NEEDS_OVERWRITE=true
    fi

    if [ "$NEEDS_OVERWRITE" = "true" ] && [ -z "''${NIXFIED_INSTALL_FORCE:-}" ]; then
      if [ -t 0 ]; then
        if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
          echo "⚠️  Existing Nixfied install found in $ROOT"
          echo "    This will upgrade framework files and preserve nixfied/project/"
          echo "    It will overwrite: flake.nix, flake.lock, nixfied/ (except nixfied/project/)"
        else
          echo "⚠️  Existing Nix files found in $ROOT"
          echo "    This will overwrite: flake.nix, flake.lock, nixfied/"
        fi
        echo -n "Continue? [y/N]: "
        read -r REPLY
        if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
          echo "Aborted."
          exit 1
        fi
      else
        echo "❌ Existing Nix files found. Re-run with NIXFIED_INSTALL_FORCE=1 to overwrite." >&2
        exit 1
      fi
    fi

    echo "📦 Installing framework files..."

    cp -f "$SRC/flake.nix" "$ROOT/flake.nix"
    if [ -f "$SRC/flake.lock" ]; then
      cp -f "$SRC/flake.lock" "$ROOT/flake.lock"
    fi

    PRESERVE_PROJECT=false
    if [ -d "$ROOT/nixfied/project" ] && [ "$RESET_PROJECT" != "true" ]; then
      PRESERVE_PROJECT=true
    fi
    if [ "$MODE" = "upgrade" ] && [ "$RESET_PROJECT" != "true" ]; then
      PRESERVE_PROJECT=true
    fi

    if [ -n "$FILTERS_RAW" ] && [ "$PRESERVE_PROJECT" = "true" ]; then
      echo "ℹ️  Skipping --filter on upgrade (nixfied/project is preserved)."
      FILTERS_RAW=""
    fi

    if [ -d "$ROOT/nixfied" ]; then
      chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
    fi

    if [ "$PRESERVE_PROJECT" = "true" ]; then
      echo "🔧 Upgrading nixfied/ (preserving nixfied/project/)"
      ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w --exclude='/project/' "$SRC/nixfied/" "$ROOT/nixfied/"
    else
      ${pkgs.rsync}/bin/rsync -a --delete --chmod=Du+w,Fu+w "$SRC/nixfied/" "$ROOT/nixfied/"
    fi

    chmod -R u+w "$ROOT/nixfied" 2>/dev/null || true
    if command -v chflags >/dev/null 2>&1; then
      chflags -R nouchg "$ROOT/nixfied" 2>/dev/null || true
    fi
    if command -v chattr >/dev/null 2>&1; then
      chattr -R -i "$ROOT/nixfied" 2>/dev/null || true
    fi
    chmod u+w "$ROOT/nixfied/.framework" 2>/dev/null || true
    if command -v chflags >/dev/null 2>&1; then
      chflags nouchg "$ROOT/nixfied/.framework" 2>/dev/null || true
    fi
    if command -v chattr >/dev/null 2>&1; then
      chattr -i "$ROOT/nixfied/.framework" 2>/dev/null || true
    fi
    rm -f "$ROOT/nixfied/.framework"

    if [ -n "$FILTERS_RAW" ]; then
      IFS=',' read -r -a FILTERS <<< "$FILTERS_RAW"
      declare -A KEEP
      KEEP[conf]=1

      for f in "''${FILTERS[@]}"; do
        f="''${f,,}"
        case "$f" in
          build)
            KEEP[prod]=1
            ;;
          conf|dev|test|prod|quality|ci)
            KEEP["$f"]=1
            ;;
          "")
            ;;
          *)
            echo "❌ Unknown filter: $f" >&2
            exit 1
            ;;
        esac
      done

      for f in dev test prod quality ci; do
        if [ -z "''${KEEP[$f]:-}" ]; then
          rm -f "$ROOT/nixfied/project/$f.nix" 2>/dev/null || true
        fi
      done

      {
        echo "{ pkgs ? null }:"
        echo ""
        echo "let"
        echo "  conf = import ./conf.nix { inherit pkgs; };"
        echo "  project = conf.project or { };"
        echo "  parts = ["
        echo "    conf"
        for f in dev test prod quality ci; do
          if [ -n "''${KEEP[$f]:-}" ]; then
            echo "    (import ./$f.nix { inherit pkgs project; })"
          fi
        done
        echo "  ];"
        echo "in"
        echo "pkgs.lib.foldl' pkgs.lib.recursiveUpdate { } parts"
      } > "$ROOT/nixfied/project/default.nix"
    fi

    if [ "$PROMPT_PLAN" = "true" ]; then
      PLAN_EXIT=0
      set +e
      if [ "$PROMPT_PLAN_FORCE" = "true" ]; then
        ${promptPlanScript} --force
      else
        ${promptPlanScript}
      fi
      PLAN_EXIT=$?
      set -e
      if [ "$PLAN_EXIT" -ne 0 ]; then
        echo "⚠️  Prompt plan generation failed or was skipped."
      fi
    fi

    if [ "$PRESERVE_PROJECT" = "true" ] || [ "$MODE" = "upgrade" ]; then
      echo "✅ Framework upgraded."
      echo "    Preserved nixfied/project/ (pass --reset-project to overwrite)."
    else
      echo "✅ Framework installed."
    fi
    echo "Next:"
    echo "  - Edit nixfied/project/conf.nix"
    echo "  - Customize nixfied/project/{dev,test,prod,quality,ci}.nix (prod.nix defines the build command)"
  '';
in
{
  install = mkApp {
    name = "install";
    description = "Install Nixfied framework into a repository";
    env = { };
    useDeps = false;
    script = installScript;
  };

  upgrade = mkApp {
    name = "upgrade";
    description = "Upgrade Nixfied framework in-place (preserving nixfied/project by default)";
    env = {
      NIXFIED_INSTALL_MODE = "upgrade";
    };
    useDeps = false;
    script = installScript;
  };

  "prompt-plan" = mkApp {
    name = "prompt-plan";
    description = "Generate Nixfied prompt plan from project docs";
    env = { };
    useDeps = false;
    script = ''
      ${promptPlanScript} "$@"
    '';
  };
}
