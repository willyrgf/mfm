# Framework test runner app
{ pkgs, lib }:

let
  extraPath = pkgs.lib.makeBinPath [
    pkgs.bash
    pkgs.coreutils
    pkgs.findutils
    pkgs.gawk
    pkgs.gnugrep
    pkgs.gnused
    pkgs.git
    pkgs.nix
    pkgs.rsync
  ];
  testScript = pkgs.writeShellScript "framework-test" ''
    set -euo pipefail

    ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
    ROOT=$(cd "$ROOT" && pwd -P)
    cd "$ROOT"

    if [ ! -f "$ROOT/flake.nix" ] || [ ! -d "$ROOT/nixfied" ]; then
      echo "Run from the framework repository root." >&2
      exit 1
    fi

    if ! command -v nix >/dev/null 2>&1; then
      echo "nix is required in PATH" >&2
      exit 1
    fi

    if ! command -v git >/dev/null 2>&1; then
      echo "git is required in PATH" >&2
      exit 1
    fi

    WORKDIR=$(mktemp -d)
    WORKDIR=$(cd "$WORKDIR" && pwd -P)
    SYSTEM="${pkgs.stdenv.hostPlatform.system}"

    cleanup() {
      if [ "''${NIXFIED_TEST_KEEP:-}" = "1" ]; then
        echo "Keeping test workspace: $WORKDIR"
      else
        rm -rf "$WORKDIR"
      fi
    }
    trap cleanup EXIT

    export XDG_DATA_HOME="$WORKDIR/xdg"

    log() {
      printf '%s\n' "==> $*"
    }

    fail() {
      printf '%s\n' "FAIL: $*" >&2
      exit 1
    }

    assert_file_exists() {
      local path="$1"
      [ -f "$path" ] || fail "expected file: $path"
    }

    assert_file_absent() {
      local path="$1"
      [ ! -e "$path" ] || fail "unexpected file: $path"
    }

    assert_contains() {
      local file="$1"
      local pattern="$2"
      grep -q "$pattern" "$file" || fail "expected '$pattern' in $file"
    }

    run_app() {
      local flake_path="$1"
      local app="$2"
      shift 2
      if [ "$#" -gt 0 ]; then
        nix run "path:$flake_path"#"$app" -- "$@"
      else
        nix run "path:$flake_path"#"$app"
      fi
    }

    run_app_quiet() {
      run_app "$@" >/dev/null
    }

    assert_app_missing() {
      local flake_path="$1"
      local app="$2"
      set +e
      nix run "path:$flake_path"#"$app" >/dev/null 2>&1
      local rc=$?
      set -e
      if [ "$rc" -eq 0 ]; then
        fail "expected app to be missing: $app"
      fi
    }

    build_expr() {
      local expr="$1"
      nix build --impure --expr "$expr" --argstr root "$ROOT" --argstr system "$SYSTEM" --no-link --print-out-paths
    }

    init_repo() {
      local dir="$1"
      mkdir -p "$dir"
      (
        cd "$dir" \
          && git init -q \
          && git config user.email "nixfied-test@example.invalid" \
          && git config user.name "nixfied test"
      )
    }

    log "flake eval"
    nix flake show "path:$ROOT" >/dev/null
    nix flake check --no-build "path:$ROOT" >/dev/null

    log "core apps"
    HELP_OUT="$WORKDIR/help.txt"
    nix run "path:$ROOT"#help > "$HELP_OUT"
    assert_contains "$HELP_OUT" "Commands:"
    assert_contains "$HELP_OUT" "PROJECT_ENV"
    assert_contains "$HELP_OUT" "NIX_ENV"
    assert_contains "$HELP_OUT" "NIXFIED_ENV"

    HELP_DEV_OUT="$WORKDIR/help-dev.txt"
    nix run "path:$ROOT"#help -- dev > "$HELP_DEV_OUT"
    assert_contains "$HELP_DEV_OUT" "Usage:"
    assert_contains "$HELP_DEV_OUT" "nix run .#dev"

    nix run "path:$ROOT"#dev >/dev/null
    nix run "path:$ROOT"#test >/dev/null
    nix run "path:$ROOT"#build >/dev/null
    nix run "path:$ROOT"#check >/dev/null
    nix run "path:$ROOT"#ci -- --summary >/dev/null

    log "helpers runtime"
    HELPERS_DIR="$WORKDIR/helpers"
    mkdir -p "$HELPERS_DIR"
    HELPERS_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gnugrep
            pkgs.lsof
            pkgs.netcat
            pkgs.python3
          ];
      };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "helpers-runtime";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/helpers/runtime.nix { };
      }
    NIX
    )

    HELPERS_SCRIPT=$(build_expr "$HELPERS_EXPR")
    HELPERS_LOG="$WORKDIR/helpers-runtime.log"
    set +e
    (cd "$HELPERS_DIR" && "$HELPERS_SCRIPT" >"$HELPERS_LOG" 2>&1)
    HELPERS_RC=$?
    set -e
    if [ "$HELPERS_RC" -ne 0 ]; then
      echo "Helpers runtime fixture failed (rc=$HELPERS_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 100 lines):" >&2
      tail -100 "$HELPERS_LOG" >&2 || true
      exit "$HELPERS_RC"
    fi

    log "slots env"
    SLOTS_DIR="$WORKDIR/slots"
    mkdir -p "$SLOTS_DIR"
    SLOTS_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gnugrep
            pkgs.python3
          ];
      };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "slots-runtime";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/slots/runtime.nix { };
      }
    NIX
    )

    SLOTS_SCRIPT=$(build_expr "$SLOTS_EXPR")
    SLOTS_LOG="$WORKDIR/slots-runtime.log"
    set +e
    (cd "$SLOTS_DIR" && "$SLOTS_SCRIPT" >"$SLOTS_LOG" 2>&1)
    SLOTS_RC=$?
    set -e
    if [ "$SLOTS_RC" -ne 0 ]; then
      echo "Slots env fixture failed (rc=$SLOTS_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 100 lines):" >&2
      tail -100 "$SLOTS_LOG" >&2 || true
      exit "$SLOTS_RC"
    fi

    log "ci DSL fixture"
    CI_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      fixture = import ./tests/framework/fixtures/ci/ci.nix { project = base.project; };
      project = pkgs.lib.recursiveUpdate base fixture;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_SCRIPT=$(build_expr "$CI_EXPR")
    CI_BASIC_DIR="$WORKDIR/ci-basic"
    CI_BASIC_LOG="$WORKDIR/ci-basic.log"
    mkdir -p "$CI_BASIC_DIR"
    unset CI_MISSING
    set +e
    (cd "$CI_BASIC_DIR" && "$CI_SCRIPT" --mode basic > "$CI_BASIC_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      fail "expected CI basic mode to exit zero"
    fi
    assert_file_exists "$CI_BASIC_DIR/.ci-artifacts/runs.ok"
    assert_file_absent "$CI_BASIC_DIR/.ci-artifacts/skip-missing.ok"
    assert_file_absent "$CI_BASIC_DIR/.ci-artifacts/when.ok"
    assert_file_absent "$CI_BASIC_DIR/.ci-artifacts/requires.ok"
    assert_file_exists "$CI_BASIC_DIR/.ci-artifacts/teardown.ok"
    assert_contains "$CI_BASIC_LOG" "missing CI_MISSING"
    assert_contains "$CI_BASIC_LOG" "condition not met"
    assert_contains "$CI_BASIC_LOG" "requires module(s): nginx"

    CI_FAIL_DIR="$WORKDIR/ci-failure"
    CI_FAIL_LOG="$WORKDIR/ci-failure.log"
    mkdir -p "$CI_FAIL_DIR"
    set +e
    (cd "$CI_FAIL_DIR" && "$CI_SCRIPT" --mode failure > "$CI_FAIL_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected CI failure mode to exit non-zero"
    fi
    assert_file_exists "$CI_FAIL_DIR/.ci-artifacts/fail.cleanup"
    assert_file_exists "$CI_FAIL_DIR/.ci-artifacts/teardown.ok"

    CI_ERR_DIR="$WORKDIR/ci-errors"
    CI_MODE_LOG="$WORKDIR/ci-unknown-mode.log"
    CI_FLAG_LOG="$WORKDIR/ci-unknown-flag.log"
    mkdir -p "$CI_ERR_DIR"
    set +e
    (cd "$CI_ERR_DIR" && "$CI_SCRIPT" --mode does-not-exist > "$CI_MODE_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected unknown CI mode to exit non-zero"
    fi
    assert_contains "$CI_MODE_LOG" "Unknown CI mode"

    set +e
    (cd "$CI_ERR_DIR" && "$CI_SCRIPT" --no-such-flag > "$CI_FLAG_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected unknown CI flag to exit non-zero"
    fi
    assert_contains "$CI_FLAG_LOG" "Unknown option"

    log "ci artifacts retention"
    CI_RET_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      fixture = import ./tests/framework/fixtures/ci/retention.nix { project = base.project; };
      project = pkgs.lib.recursiveUpdate base fixture;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_RET_SCRIPT=$(build_expr "$CI_RET_EXPR")
    CI_RET_OK_DIR="$WORKDIR/ci-ret-ok"
    CI_RET_FAIL_DIR="$WORKDIR/ci-ret-fail"
    mkdir -p "$CI_RET_OK_DIR" "$CI_RET_FAIL_DIR"
    (cd "$CI_RET_OK_DIR" && "$CI_RET_SCRIPT" --mode success >/dev/null)
    assert_file_absent "$CI_RET_OK_DIR/.ci-artifacts"

    set +e
    (cd "$CI_RET_FAIL_DIR" && "$CI_RET_SCRIPT" --mode failure >/dev/null 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected retention failure mode to exit non-zero"
    fi
    assert_file_exists "$CI_RET_FAIL_DIR/.ci-artifacts/fail.cleanup"

    log "ci unknown step"
    CI_UNKNOWN_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      fixture = import ./tests/framework/fixtures/ci/unknown-step.nix { project = base.project; };
      project = pkgs.lib.recursiveUpdate base fixture;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_UNKNOWN_SCRIPT=$(build_expr "$CI_UNKNOWN_EXPR")
    CI_UNKNOWN_DIR="$WORKDIR/ci-unknown-step"
    CI_UNKNOWN_LOG="$WORKDIR/ci-unknown-step.log"
    mkdir -p "$CI_UNKNOWN_DIR"
    set +e
    (cd "$CI_UNKNOWN_DIR" && "$CI_UNKNOWN_SCRIPT" > "$CI_UNKNOWN_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected unknown step to exit non-zero"
    fi
    assert_contains "$CI_UNKNOWN_LOG" "Unknown step"

    CI_RET_SUM_OK_DIR="$WORKDIR/ci-ret-summary-ok"
    CI_RET_SUM_OK_LOG="$WORKDIR/ci-ret-summary-ok.log"
    mkdir -p "$CI_RET_SUM_OK_DIR"
    (cd "$CI_RET_SUM_OK_DIR" && "$CI_RET_SCRIPT" --mode success --summary > "$CI_RET_SUM_OK_LOG" 2>&1)
    assert_contains "$CI_RET_SUM_OK_LOG" "Summary"
    assert_contains "$CI_RET_SUM_OK_LOG" "Exit code: 0"
    assert_file_absent "$CI_RET_SUM_OK_DIR/.ci-artifacts"

    CI_RET_SUM_FAIL_DIR="$WORKDIR/ci-ret-summary-fail"
    CI_RET_SUM_FAIL_LOG="$WORKDIR/ci-ret-summary-fail.log"
    mkdir -p "$CI_RET_SUM_FAIL_DIR"
    set +e
    (cd "$CI_RET_SUM_FAIL_DIR" && "$CI_RET_SCRIPT" --mode failure --summary > "$CI_RET_SUM_FAIL_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected retention failure summary mode to exit non-zero"
    fi
    assert_contains "$CI_RET_SUM_FAIL_LOG" "Summary"
    assert_contains "$CI_RET_SUM_FAIL_LOG" "Exit code: 1"
    assert_contains "$CI_RET_SUM_FAIL_LOG" "Last 50 lines"
    assert_file_exists "$CI_RET_SUM_FAIL_DIR/.ci-artifacts/fail.cleanup"

    log "module hooks fixture"
    MOD_DIR="$WORKDIR/modules"
    mkdir -p "$MOD_DIR"
    MOD_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      dev = import ./tests/framework/fixtures/modules/dev.nix { project = conf.project; };
      project = pkgs.lib.recursiveUpdate base (pkgs.lib.recursiveUpdate conf dev);
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      postgres =
        if (project.modules.postgres.enable or false) then
          import ./nixfied/postgres { inherit pkgs project slots; }
        else
          null;
      nginx =
        if (project.modules.nginx.enable or false) then
          import ./nixfied/nginx { inherit pkgs project slots; }
        else
          null;
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots postgres nginx; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
    in
      let
        devCfg = dev.commands.dev or { };
      in
      lib.mkAppScript {
        name = "dev";
        script = devCfg.script or "";
        env = devCfg.env or { };
        useDeps = devCfg.useDeps or false;
      }
    NIX
    )

    DEV_SCRIPT=$(build_expr "$MOD_EXPR")
    DEV_LOG="$WORKDIR/dev-fixture.log"
    set +e
    (
      unset SLOT_INFO REQUIRE_SLOT_ENV \
        POSTGRES_INIT POSTGRES_START POSTGRES_STOP POSTGRES_SETUP_DB POSTGRES_FULL_START POSTGRES_FULL_START_TEST \
        NGINX_INIT NGINX_START NGINX_STOP NGINX_SITE_PROXY NGINX_SITE_STATIC
      export NIXFIED_TEST_DEBUG=1
      cd "$MOD_DIR" && "$DEV_SCRIPT" >"$DEV_LOG" 2>&1
    )
    DEV_RC=$?
    set -e
    if [ "$DEV_RC" -ne 0 ]; then
      echo "Module fixture failed (rc=$DEV_RC)." >&2
      echo "" >&2
      echo "Script excerpt:" >&2
      nl -ba "$DEV_SCRIPT" | sed -n '60,100p' >&2
      echo "" >&2
      echo "Fixture output (last 50 lines):" >&2
      tail -50 "$DEV_LOG" >&2 || true
      echo "" >&2
      echo "Matches for 'name}' in script:" >&2
      grep -n "name}" "$DEV_SCRIPT" >&2 || true
      exit "$DEV_RC"
    fi

    log "supervisor config"
    SUP_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        supervisor = {
          enable = true;
          services = {
            app = {
              command = "echo hello";
              workingDir = ".";
            };
          };
        };
      };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/supervisor { inherit pkgs project slots; };
    in
      supervisor.generateConfig
    NIX
    )

    SUP_SCRIPT=$(build_expr "$SUP_EXPR")
    SUP_CONFIG=$("$SUP_SCRIPT")
    assert_file_exists "$SUP_CONFIG"
    assert_contains "$SUP_CONFIG" "processes:"
    assert_contains "$SUP_CONFIG" "app:"

    export NIXFIED_PROMPT_PLAN=0
    log "installer basic"
    INSTALL_BASE="$WORKDIR/install-repo"
    init_repo "$INSTALL_BASE"
    (cd "$INSTALL_BASE" && nix run "path:$ROOT"#framework::install >/dev/null)
    INSTALL_TARGET="$INSTALL_BASE"
    INSTALL_BRANCH=$(git -C "$INSTALL_TARGET" symbolic-ref --short HEAD 2>/dev/null || git -C "$INSTALL_TARGET" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    if [ "$INSTALL_BRANCH" != "nixfied" ]; then
      fail "expected installer to switch to nixfied branch (got: $INSTALL_BRANCH)"
    fi
    assert_file_exists "$INSTALL_TARGET/flake.nix"
    if [ ! -d "$INSTALL_TARGET/nixfied" ]; then
      fail "expected nixfied/ directory in installer target"
    fi
    assert_file_absent "$INSTALL_TARGET/nixfied/.framework"
    assert_file_absent "$INSTALL_TARGET/NIXFIED_PROMPT_PLAN.md"
    assert_file_exists "$INSTALL_TARGET/nixfied/local/default.nix"

    log "installer worktree"
    INSTALL_WT_BASE="$WORKDIR/install-worktree"
    init_repo "$INSTALL_WT_BASE"
    echo "test" > "$INSTALL_WT_BASE/README.md"
    (cd "$INSTALL_WT_BASE" && git add README.md && git commit -qm "init")
    INSTALL_WT_TARGET="$WORKDIR/install-worktree-target"
    ORIG_BRANCH=$(git -C "$INSTALL_WT_BASE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    (cd "$INSTALL_WT_BASE" && nix run "path:$ROOT"#framework::install -- --worktree --target "$INSTALL_WT_TARGET" --force >/dev/null)
    AFTER_ORIG_BRANCH=$(git -C "$INSTALL_WT_BASE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    if [ "$AFTER_ORIG_BRANCH" != "$ORIG_BRANCH" ]; then
      fail "expected worktree install to keep current checkout on '$ORIG_BRANCH' (got: $AFTER_ORIG_BRANCH)"
    fi
    if ! git -C "$INSTALL_WT_BASE" worktree list | grep -q "$INSTALL_WT_TARGET"; then
      fail "expected git worktree to exist: $INSTALL_WT_TARGET"
    fi
    WT_BRANCH=$(git -C "$INSTALL_WT_TARGET" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    if [ "$WT_BRANCH" != "nixfied" ]; then
      fail "expected worktree install to use nixfied branch (got: $WT_BRANCH)"
    fi
    assert_file_exists "$INSTALL_WT_TARGET/flake.nix"
    if [ ! -d "$INSTALL_WT_TARGET/nixfied" ]; then
      fail "expected nixfied/ directory in worktree target"
    fi
    assert_file_absent "$INSTALL_WT_TARGET/nixfied/.framework"
    assert_file_exists "$INSTALL_WT_TARGET/nixfied/local/default.nix"

    log "example project apps (basic install)"
    BASIC_HELP="$WORKDIR/basic-help.txt"
    run_app "$INSTALL_TARGET" help > "$BASIC_HELP"
    assert_contains "$BASIC_HELP" "Commands:"
    assert_contains "$BASIC_HELP" "dev  Start the dev workflow"
    assert_contains "$BASIC_HELP" "test  Run tests"
    assert_contains "$BASIC_HELP" "build  Build artifacts"
    assert_contains "$BASIC_HELP" "check  Run quality checks"
    assert_contains "$BASIC_HELP" "ci  Run the CI pipeline"
    BASIC_HELP_DEV="$WORKDIR/basic-help-dev.txt"
    run_app "$INSTALL_TARGET" help dev > "$BASIC_HELP_DEV"
    assert_contains "$BASIC_HELP_DEV" "Usage:"
    assert_contains "$BASIC_HELP_DEV" "nix run .#dev"
    run_app_quiet "$INSTALL_TARGET" dev
    run_app_quiet "$INSTALL_TARGET" test
    run_app_quiet "$INSTALL_TARGET" build
    run_app_quiet "$INSTALL_TARGET" check
    run_app_quiet "$INSTALL_TARGET" ci --summary
    assert_app_missing "$INSTALL_TARGET" "framework::install"
    assert_app_missing "$INSTALL_TARGET" "framework::prompt-plan"
    assert_app_missing "$INSTALL_TARGET" "framework::test"

    log "ports env var aliases"
    PORTS_ALIAS_OUT="$WORKDIR/ports-alias.txt"
    NIXFIED_ENV=1 run_app "$INSTALL_TARGET" ports > "$PORTS_ALIAS_OUT"
    assert_contains "$PORTS_ALIAS_OUT" "Port assignments for slot 1, env dev"
    PORTS_NIX_ENV_OUT="$WORKDIR/ports-nix-env.txt"
    NIX_ENV=1 run_app "$INSTALL_TARGET" ports > "$PORTS_NIX_ENV_OUT"
    assert_contains "$PORTS_NIX_ENV_OUT" "Port assignments for slot 1, env dev"

    log "app api contract enforcement"
    BAD_API_BASE="$WORKDIR/install-bad-api"
    init_repo "$BAD_API_BASE"
    (cd "$BAD_API_BASE" && nix run "path:$ROOT"#framework::install >/dev/null)
    BAD_API_TARGET="$BAD_API_BASE"
    assert_file_exists "$BAD_API_TARGET/flake.nix"
    cat > "$BAD_API_TARGET/nixfied/project/dev.nix" <<'EOF'
    { ... }:

    {
      commands = {
        dev = {
          description = "Start the dev workflow";
          api = {
            version = 1;
            summary = "Start the dev workflow";
            details = "ok";
            usage = [ "nix run .#dev" ];
          };
          env = {
            PROJECT_ENV = "dev";
          };
          useDeps = true;
          script = "echo dev\nexit 0\n";
        };

        missing-api = {
          description = "This command is missing api";
          env = { };
          useDeps = false;
          script = "echo missing\nexit 0\n";
        };
      };
    }
    EOF
    BAD_API_LOG="$WORKDIR/bad-api-contract.log"
    set +e
    nix flake show "path:$BAD_API_TARGET" > "$BAD_API_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected API contract violation to fail"
    fi
    assert_contains "$BAD_API_LOG" "Nixfied app API contract violated"
    assert_contains "$BAD_API_LOG" "missing meta.nixfied.api"

    log "installer upgrade preserves project"
    echo "# NIXFIED_UPGRADE_TEST_MARKER" >> "$INSTALL_TARGET/nixfied/project/conf.nix"
    echo "# NIXFIED_LOCAL_UPGRADE_TEST_MARKER" >> "$INSTALL_TARGET/nixfied/local/default.nix"
    (cd "$INSTALL_TARGET" && nix run "path:$ROOT"#framework::upgrade -- --force >/dev/null)
    assert_contains "$INSTALL_TARGET/nixfied/project/conf.nix" "NIXFIED_UPGRADE_TEST_MARKER"
    assert_contains "$INSTALL_TARGET/nixfied/local/default.nix" "NIXFIED_LOCAL_UPGRADE_TEST_MARKER"
    assert_file_absent "$INSTALL_TARGET/nixfied/.framework"

    log "framework marker toggle"
    touch "$INSTALL_TARGET/nixfied/.framework"
    FRAMEWORK_HELP="$WORKDIR/framework-help.txt"
    run_app "$INSTALL_TARGET" "framework::prompt-plan" --help > "$FRAMEWORK_HELP"
    assert_contains "$FRAMEWORK_HELP" "prompt-plan"
    FRAMEWORK_INSTALL_HELP="$WORKDIR/framework-install-help.txt"
    run_app "$INSTALL_TARGET" "framework::install" --help > "$FRAMEWORK_INSTALL_HELP"
    assert_contains "$FRAMEWORK_INSTALL_HELP" "framework::install"
    PROMPT_PLAN_OUT="$WORKDIR/prompt-plan-disabled.md"
    assert_file_absent "$PROMPT_PLAN_OUT"
    PROMPT_PLAN_LOG="$WORKDIR/prompt-plan-disabled.log"
    set +e
    NIXFIED_PROMPT_PLAN=0 run_app "$INSTALL_TARGET" "framework::prompt-plan" -- --output="$PROMPT_PLAN_OUT" > "$PROMPT_PLAN_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      fail "expected prompt-plan disabled run to exit zero"
    fi
    assert_contains "$PROMPT_PLAN_LOG" "Prompt plan disabled"
    assert_file_absent "$PROMPT_PLAN_OUT"
    PROMPT_PLAN_FORCE_OUT="$WORKDIR/prompt-plan-enabled.md"
    assert_file_absent "$PROMPT_PLAN_FORCE_OUT"
    NIXFIED_PROMPT_PLAN=1 run_app "$INSTALL_TARGET" "framework::prompt-plan" -- --force --output="$PROMPT_PLAN_FORCE_OUT" >/dev/null
    assert_file_exists "$PROMPT_PLAN_FORCE_OUT"
    rm -f "$INSTALL_TARGET/nixfied/.framework"

    log "installer re-entry"
    REENTRY_BASE="$WORKDIR/install-reentry"
    init_repo "$REENTRY_BASE"
    (cd "$REENTRY_BASE" && nix run "path:$ROOT"#framework::install >/dev/null)
    REENTRY_TARGET="$REENTRY_BASE"
    assert_file_exists "$REENTRY_TARGET/flake.nix"
    assert_file_absent "$REENTRY_TARGET/nixfied/.framework"
    (cd "$REENTRY_BASE" && nix run "path:$ROOT"#framework::install -- --force >/dev/null)
    assert_file_exists "$REENTRY_TARGET/flake.nix"
    assert_file_absent "''${REENTRY_BASE}_nixfied"
    assert_file_absent "''${REENTRY_BASE}_nixified"

    log "installer filter"
    INSTALL_FILTER="$WORKDIR/install-filter"
    init_repo "$INSTALL_FILTER"
    (cd "$INSTALL_FILTER" && nix run "path:$ROOT"#framework::install -- --filter=conf,ci >/dev/null)
    FILTER_TARGET="$INSTALL_FILTER"
    assert_file_exists "$FILTER_TARGET/nixfied/project/ci.nix"
    assert_file_absent "$FILTER_TARGET/nixfied/project/dev.nix"
    assert_file_absent "$FILTER_TARGET/nixfied/project/test.nix"
    assert_file_absent "$FILTER_TARGET/nixfied/project/prod.nix"
    assert_file_absent "$FILTER_TARGET/nixfied/project/quality.nix"
    if grep -q "dev.nix" "$FILTER_TARGET/nixfied/project/default.nix"; then
      fail "default.nix should not include dev.nix when filtered"
    fi
    assert_file_absent "$FILTER_TARGET/nixfied/.framework"
    assert_file_absent "$FILTER_TARGET/NIXFIED_PROMPT_PLAN.md"

    log "example project apps (filtered install)"
    FILTER_HELP="$WORKDIR/filter-help.txt"
    run_app "$FILTER_TARGET" help > "$FILTER_HELP"
    assert_contains "$FILTER_HELP" "Commands:"
    assert_contains "$FILTER_HELP" "ci  Run the CI pipeline"
    assert_app_missing "$FILTER_TARGET" "framework::install"
    assert_app_missing "$FILTER_TARGET" "framework::prompt-plan"
    assert_app_missing "$FILTER_TARGET" "dev"
    assert_app_missing "$FILTER_TARGET" "test"
    assert_app_missing "$FILTER_TARGET" "build"
    assert_app_missing "$FILTER_TARGET" "check"
    run_app_quiet "$FILTER_TARGET" ci --summary

    log "installer invalid filter"
    INSTALL_BAD_FILTER="$WORKDIR/install-bad-filter"
    init_repo "$INSTALL_BAD_FILTER"
    BAD_FILTER_LOG="$WORKDIR/install-bad-filter.log"
    set +e
    (cd "$INSTALL_BAD_FILTER" && nix run "path:$ROOT"#framework::install -- --filter=conf,nope > "$BAD_FILTER_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected invalid filter to exit non-zero"
    fi
    assert_contains "$BAD_FILTER_LOG" "Unknown filter"

    log "installer force"
    INSTALL_FORCE="$WORKDIR/force_nixified"
    init_repo "$INSTALL_FORCE"
    mkdir -p "$INSTALL_FORCE/nixfied"
    (cd "$INSTALL_FORCE" && nix run "path:$ROOT"#framework::install -- --force >/dev/null)
    assert_file_exists "$INSTALL_FORCE/flake.nix"
    if [ ! -d "$INSTALL_FORCE/nixfied" ]; then
      fail "expected nixfied/ directory in force target"
    fi
    assert_file_absent "$INSTALL_FORCE/nixfied/.framework"
    assert_file_absent "$INSTALL_FORCE/NIXFIED_PROMPT_PLAN.md"

    log "example project apps (force install)"
    FORCE_HELP="$WORKDIR/force-help.txt"
    run_app "$INSTALL_FORCE" help > "$FORCE_HELP"
    assert_contains "$FORCE_HELP" "Commands:"
    assert_contains "$FORCE_HELP" "dev  Start the dev workflow"
    assert_contains "$FORCE_HELP" "test  Run tests"
    assert_contains "$FORCE_HELP" "build  Build artifacts"
    assert_contains "$FORCE_HELP" "check  Run quality checks"
    assert_contains "$FORCE_HELP" "ci  Run the CI pipeline"
    run_app_quiet "$INSTALL_FORCE" dev
    run_app_quiet "$INSTALL_FORCE" test
    run_app_quiet "$INSTALL_FORCE" build
    run_app_quiet "$INSTALL_FORCE" check
    run_app_quiet "$INSTALL_FORCE" ci --summary
    assert_app_missing "$INSTALL_FORCE" "framework::install"
    assert_app_missing "$INSTALL_FORCE" "framework::prompt-plan"
    assert_app_missing "$INSTALL_FORCE" "framework::test"

    log "ephemeral slot locking"
    EPHEM_DIR="$WORKDIR/ephemeral"
    mkdir -p "$EPHEM_DIR"
    EPHEM_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        ephemeral.enable = true;
      };
      ephemeral = import ./nixfied/ephemeral.nix { inherit pkgs project; };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "ephemeral-lock-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/ephemeral/lock.nix {
          acquireSlotLock = toString ephemeral.acquireSlotLock;
          releaseSlotLock = toString ephemeral.releaseSlotLock;
          projectIdUpper = "NIXFIED_PROJECT";
        };
      }
    NIX
    )

    EPHEM_SCRIPT=$(build_expr "$EPHEM_EXPR")
    EPHEM_LOG="$WORKDIR/ephemeral-lock.log"
    set +e
    (cd "$EPHEM_DIR" && "$EPHEM_SCRIPT" >"$EPHEM_LOG" 2>&1)
    EPHEM_RC=$?
    set -e
    if [ "$EPHEM_RC" -ne 0 ]; then
      echo "Ephemeral lock fixture failed (rc=$EPHEM_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 50 lines):" >&2
      tail -50 "$EPHEM_LOG" >&2 || true
      exit "$EPHEM_RC"
    fi

    log "run registry foreground"
    REG_DIR="$WORKDIR/registry"
    mkdir -p "$REG_DIR"
    REG_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = base;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "registry-fg-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/registry/foreground.nix {
          runRegistryStart = toString lib.runRegistryStart;
          runsRoot = "/tmp/nixfied-project-runs";
        };
      }
    NIX
    )

    REG_SCRIPT=$(build_expr "$REG_EXPR")
    REG_LOG="$WORKDIR/registry-fg.log"
    set +e
    (cd "$REG_DIR" && "$REG_SCRIPT" >"$REG_LOG" 2>&1)
    REG_RC=$?
    set -e
    if [ "$REG_RC" -ne 0 ]; then
      echo "Registry foreground fixture failed (rc=$REG_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 50 lines):" >&2
      tail -50 "$REG_LOG" >&2 || true
      exit "$REG_RC"
    fi

    log "ci summary.json"
    CI_SJ_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      fixture = import ./tests/framework/fixtures/ci/summary-json.nix { project = base.project; };
      project = pkgs.lib.recursiveUpdate base fixture;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_SJ_SCRIPT=$(build_expr "$CI_SJ_EXPR")
    CI_SJ_DIR="$WORKDIR/ci-summary-json"
    CI_SJ_LOG="$WORKDIR/ci-summary-json.log"
    mkdir -p "$CI_SJ_DIR"
    set +e
    (cd "$CI_SJ_DIR" && "$CI_SJ_SCRIPT" --mode check > "$CI_SJ_LOG" 2>&1)
    CI_SJ_RC=$?
    set -e
    if [ "$CI_SJ_RC" -ne 0 ]; then
      fail "expected CI summary-json mode to exit zero"
    fi
    assert_file_exists "$CI_SJ_DIR/.ci-artifacts/summary.json"
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"mode"'
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"check"'
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"exit_code": 0'
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"passing"'
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"passed"'
    assert_contains "$CI_SJ_DIR/.ci-artifacts/summary.json" '"skipped"'

    log "module apps exposure"
    MODAPP_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules.postgres.enable = true;
        modules.nginx.enable = true;
      };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      postgres = import ./nixfied/postgres { inherit pkgs project slots; };
      nginx = import ./nixfied/nginx { inherit pkgs project slots; };
      supervisor = import ./nixfied/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/hooks.nix {
        inherit pkgs project slots postgres nginx supervisor;
      };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      moduleApps = import ./nixfied/internal/module-apps.nix {
        inherit pkgs project lib postgres nginx supervisor slots;
      };
    in
      pkgs.writeText "module-app-names" (builtins.concatStringsSep "\n" (builtins.attrNames moduleApps))
    NIX
    )

    MODAPP_NAMES_FILE=$(build_expr "$MODAPP_EXPR")
    assert_contains "$MODAPP_NAMES_FILE" "db-start"
    assert_contains "$MODAPP_NAMES_FILE" "nginx-start"
    assert_contains "$MODAPP_NAMES_FILE" "up"
    assert_contains "$MODAPP_NAMES_FILE" "check-ports"

    MODAPP_DISABLED_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = base;
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/hooks.nix {
        inherit pkgs project slots supervisor;
        postgres = null;
        nginx = null;
      };
      lib = import ./nixfied/lib { inherit pkgs project hooks; };
      moduleApps = import ./nixfied/internal/module-apps.nix {
        inherit pkgs project lib slots;
        postgres = null;
        nginx = null;
        supervisor = supervisor;
      };
    in
      pkgs.writeText "module-app-names-disabled" (builtins.concatStringsSep "\n" (builtins.attrNames moduleApps))
    NIX
    )

    MODAPP_DISABLED_FILE=$(build_expr "$MODAPP_DISABLED_EXPR")
    if grep -q "db-start" "$MODAPP_DISABLED_FILE"; then
      fail "db-start should not be present when postgres is disabled"
    fi
    if grep -q "nginx-start" "$MODAPP_DISABLED_FILE"; then
      fail "nginx-start should not be present when nginx is disabled"
    fi
    assert_contains "$MODAPP_DISABLED_FILE" "check-ports"
    assert_contains "$MODAPP_DISABLED_FILE" "ports"

    log "supervisor hooks"
    SUP_HOOKS_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        supervisor = {
          enable = true;
          services = {
            app = {
              command = "echo hello";
              workingDir = ".";
            };
          };
        };
      };
      slots = import ./nixfied/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/hooks.nix {
        inherit pkgs project slots supervisor;
        postgres = null;
        nginx = null;
      };
    in
      pkgs.writeText "supervisor-hooks" (builtins.concatStringsSep "\n" (
        builtins.map (name: "''${name}=''${hooks.env.''${name}}") (
          builtins.filter (n: builtins.substring 0 10 n == "SUPERVISOR") (builtins.attrNames hooks.env)
        )
      ))
    NIX
    )

    SUP_HOOKS_FILE=$(build_expr "$SUP_HOOKS_EXPR")
    assert_contains "$SUP_HOOKS_FILE" "SUPERVISOR_START="
    assert_contains "$SUP_HOOKS_FILE" "SUPERVISOR_STOP="
    assert_contains "$SUP_HOOKS_FILE" "SUPERVISOR_STATUS="
    # Verify they point to nix store paths
    assert_contains "$SUP_HOOKS_FILE" "/nix/store/"

    if [ "''${FRAMEWORK_ISOLATION:-}" = "1" ]; then
      log "isolation runner"
      run_app "$ROOT" test-isolation
    fi

    log "all tests passed"
  '';

in
{
  test = lib.appApi.mkNixfiedApp {
    name = "test";
    api = {
      version = 1;
      summary = "Run framework integration tests";
      details = "Runs the Nixfied framework integration test suite (intended for framework development).";
      usage = [ "nix run .#framework::test" ];
      category = "framework";
    };
    env = { };
    useDeps = false;
    script = ''
      export PATH="${extraPath}:$PATH"
      ${pkgs.bash}/bin/bash ${testScript} "$@"
    '';
  };
}
