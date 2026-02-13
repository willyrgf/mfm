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

    PROFILE="full"
    SUMMARY_JSON=""

    usage() {
      cat <<'EOF'
    Usage: nix run .#framework::test [--profile ci|full] [--summary-json <path>]

    Options:
      --profile <name>      Test profile to run (ci|full). Default: full.
      --summary-json <path> Write a compact JSON summary to <path>.
      --help                Show this help.
    EOF
    }

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --profile)
          PROFILE="''${2:-}"
          if [ -z "$PROFILE" ]; then
            echo "Missing value for --profile" >&2
            exit 1
          fi
          case "$PROFILE" in
            ci|full) ;;
            *)
              echo "Unknown profile: $PROFILE (expected: ci|full)" >&2
              exit 1
              ;;
          esac
          shift 2
          ;;
        --summary-json)
          SUMMARY_JSON="''${2:-}"
          if [ -z "$SUMMARY_JSON" ]; then
            echo "Missing value for --summary-json" >&2
            exit 1
          fi
          shift 2
          ;;
        --help|-h)
          usage
          exit 0
          ;;
        *)
          echo "Unknown option: $1" >&2
          usage >&2
          exit 1
          ;;
      esac
    done

    TEST_STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    TEST_START_EPOCH=$(date +%s)

    WORKDIR=$(mktemp -d)
    WORKDIR=$(cd "$WORKDIR" && pwd -P)
    SYSTEM="${pkgs.stdenv.hostPlatform.system}"

    write_summary_json() {
      local rc="$1"
      local finished_at
      local duration
      finished_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
      duration=$(( $(date +%s) - TEST_START_EPOCH ))
      mkdir -p "$(dirname "$SUMMARY_JSON")"
      cat > "$SUMMARY_JSON" <<JSON
    {
      "profile": "$PROFILE",
      "exit_code": $rc,
      "duration_seconds": $duration,
      "started_at": "$TEST_STARTED_AT",
      "finished_at": "$finished_at"
    }
    JSON
      echo "INFO: wrote summary json path=$SUMMARY_JSON"
    }

    cleanup() {
      local rc=$?
      set +e
      if [ -n "$SUMMARY_JSON" ]; then
        write_summary_json "$rc"
      fi
      if [ "''${NIXFIED_TEST_KEEP:-}" = "1" ]; then
        echo "Keeping test workspace: $WORKDIR"
      else
        rm -rf "$WORKDIR"
      fi
      return "$rc"
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

    assert_not_contains() {
      local file="$1"
      local pattern="$2"
      if grep -q "$pattern" "$file"; then
        fail "unexpected '$pattern' in $file"
      fi
    }

    print_log_tail() {
      local file="$1"
      local lines="''${2:-80}"
      if [ -f "$file" ]; then
        tail -n "$lines" "$file" >&2 || true
      else
        echo "WARN: test log missing path=$file" >&2
      fi
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

    check_coverage_map() {
      local map="$ROOT/tests/framework/COVERAGE_MAP.txt"
      local framework_dir="$ROOT/nixfied/.framework"
      local missing=0
      local count=0
      local file
      local rel

      if [ ! -f "$map" ]; then
        echo "ERROR: coverage map missing path=$map" >&2
        exit 1
      fi

      if [ ! -d "$framework_dir" ]; then
        echo "ERROR: framework directory missing path=$framework_dir" >&2
        exit 1
      fi

      while IFS= read -r file; do
        rel="''${file#$ROOT/}"
        count=$((count + 1))
        if ! grep -Fq "\`$rel\`" "$map"; then
          echo "ERROR: coverage map missing entry file=$rel" >&2
          missing=$((missing + 1))
        fi
      done < <(find "$framework_dir" -type f -name '*.nix' | sort)

      if [ "$missing" -ne 0 ]; then
        echo "ERROR: coverage map incomplete missing=$missing total=$count" >&2
        exit 1
      fi

      echo "OK: coverage map complete total=$count"
    }

    require_contains() {
      local file="$1"
      local needle="$2"
      local check="$3"
      if ! grep -Fq "$needle" "$file"; then
        echo "ERROR: fixture contract missing check=$check file=$file needle=$needle" >&2
        exit 1
      fi
    }

    require_absent() {
      local file="$1"
      local needle="$2"
      local check="$3"
      if grep -Fq "$needle" "$file"; then
        echo "ERROR: fixture contract violation check=$check file=$file needle=$needle" >&2
        exit 1
      fi
    }

    check_fixture_hardening_contracts() {
      local reth_file="$ROOT/tests/framework/fixtures/reth/lifecycle.nix"
      local minio_file="$ROOT/tests/framework/fixtures/minio/bucket-ops.nix"
      local nginx_file="$ROOT/tests/framework/fixtures/nginx/site-lifecycle.nix"
      local helios_file="$ROOT/tests/framework/fixtures/helios/lifecycle.nix"
      local modules_file="$ROOT/tests/framework/fixtures/modules/dev.nix"

      require_contains "$reth_file" 'for _ in $(seq 1 240); do' "reth readiness retry loop"
      require_contains "$minio_file" 'for _ in $(seq 1 50); do' "minio readiness retry loop"
      require_contains "$nginx_file" 'for _ in $(seq 1 80); do' "nginx readiness retry loop"
      require_contains "$helios_file" 'HELIOS_READY_OK=0' "helios readiness retry loop"
      require_contains "$modules_file" 'POSTGRES_READY_OK=0' "modules postgres readiness retry loop"
      require_contains "$modules_file" 'RETH_READY_OK=0' "modules reth readiness retry loop"
      require_contains "$modules_file" 'HELIOS_READY_OK=0' "modules helios readiness retry loop"

      require_contains "$reth_file" 'print_log_tail "$RETH_DIR/logs/reth.log" 50' "reth guarded log diagnostics"
      require_contains "$minio_file" 'print_log_tail "$MINIO_DIR/logs/minio.log" 50' "minio guarded log diagnostics"
      require_contains "$nginx_file" 'print_log_tail "$NGINX_DIR/logs/error.log" 50' "nginx guarded log diagnostics"
      require_contains "$helios_file" 'print_log_tail "$HELIOS_DIR/logs/helios.log" 50' "helios guarded log diagnostics"
      require_contains "$modules_file" 'print_log_tail "$PGDATA/postgres.log" 50' "modules postgres guarded log diagnostics"
      require_contains "$modules_file" 'print_log_tail "$RETH_DIR/logs/reth.log" 50' "modules reth guarded log diagnostics"
      require_contains "$modules_file" 'print_log_tail "$HELIOS_DIR/logs/helios.log" 50' "modules helios guarded log diagnostics"

      require_absent "$reth_file" 'tail -50 "$RETH_DIR/logs/reth.log" >&2 || true' "reth unguarded log tail"
      require_absent "$helios_file" 'tail -50 "$HELIOS_DIR/logs/helios.log" >&2 || true' "helios unguarded log tail"
      require_absent "$modules_file" 'tail -50 "$PGDATA/postgres.log" >&2 || true' "modules postgres unguarded log tail"
      require_absent "$modules_file" 'tail -50 "$RETH_DIR/logs/reth.log" >&2 || true' "modules reth unguarded log tail"
      require_absent "$modules_file" 'tail -50 "$HELIOS_DIR/logs/helios.log" >&2 || true' "modules helios unguarded log tail"
    }

    log "flake eval"
    nix flake show "path:$ROOT" >/dev/null
    nix flake check --no-build "path:$ROOT" >/dev/null

    log "coverage map"
    check_coverage_map >/dev/null

    log "fixture hardening contracts"
    check_fixture_hardening_contracts

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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
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
      print_log_tail "$HELPERS_LOG" 100
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
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
      print_log_tail "$SLOTS_LOG" 100
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/.framework/ci.nix { inherit pkgs project lib; };
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
    (cd "$CI_BASIC_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_SCRIPT" --mode basic > "$CI_BASIC_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      fail "expected CI basic mode to exit zero"
    fi
    assert_file_exists "$CI_BASIC_DIR/.ci-artifacts/runs.ok"
    assert_file_exists "$CI_BASIC_DIR/.ci-artifacts/runs-second.ok"
    assert_file_absent "$CI_BASIC_DIR/.ci-artifacts/skip-missing.ok"
    assert_file_absent "$CI_BASIC_DIR/.ci-artifacts/when.ok"
    assert_file_exists "$CI_BASIC_DIR/.ci-artifacts/teardown.ok"
    assert_contains "$CI_BASIC_LOG" "missing CI_MISSING"
    assert_contains "$CI_BASIC_LOG" "condition not met"

    CI_SCOPED_DIR="$WORKDIR/ci-runscoped"
    CI_SCOPED_LOG="$WORKDIR/ci-runscoped.log"
    mkdir -p "$CI_SCOPED_DIR"
    set +e
    (cd "$CI_SCOPED_DIR" && "$CI_SCRIPT" --mode basic > "$CI_SCOPED_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      fail "expected run-scoped CI artifacts mode to exit zero"
    fi
    CI_LATEST_LINK="$CI_SCOPED_DIR/.ci-artifacts/latest"
    if [ ! -L "$CI_LATEST_LINK" ]; then
      fail "expected latest artifact symlink"
    fi
    CI_LATEST_TARGET=$(readlink "$CI_LATEST_LINK")
    case "$CI_LATEST_TARGET" in
      "$CI_SCOPED_DIR"/.ci-artifacts/*) ;;
      *)
        fail "latest symlink should point to a run-scoped artifacts directory"
        ;;
    esac
    assert_file_exists "$CI_LATEST_TARGET/runs.ok"

    CI_FAIL_DIR="$WORKDIR/ci-failure"
    CI_FAIL_LOG="$WORKDIR/ci-failure.log"
    mkdir -p "$CI_FAIL_DIR"
    set +e
    (cd "$CI_FAIL_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_SCRIPT" --mode failure > "$CI_FAIL_LOG" 2>&1)
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/.framework/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_RET_SCRIPT=$(build_expr "$CI_RET_EXPR")
    CI_RET_OK_DIR="$WORKDIR/ci-ret-ok"
    CI_RET_FAIL_DIR="$WORKDIR/ci-ret-fail"
    mkdir -p "$CI_RET_OK_DIR" "$CI_RET_FAIL_DIR"
    (cd "$CI_RET_OK_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_RET_SCRIPT" --mode success >/dev/null)
    assert_file_absent "$CI_RET_OK_DIR/.ci-artifacts"

    set +e
    (cd "$CI_RET_FAIL_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_RET_SCRIPT" --mode failure >/dev/null 2>&1)
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/.framework/ci.nix { inherit pkgs project lib; };
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
    (cd "$CI_RET_SUM_OK_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_RET_SCRIPT" --mode success --summary > "$CI_RET_SUM_OK_LOG" 2>&1)
    assert_contains "$CI_RET_SUM_OK_LOG" "Summary"
    assert_contains "$CI_RET_SUM_OK_LOG" "Exit code: 0"
    assert_file_absent "$CI_RET_SUM_OK_DIR/.ci-artifacts"

    CI_RET_SUM_FAIL_DIR="$WORKDIR/ci-ret-summary-fail"
    CI_RET_SUM_FAIL_LOG="$WORKDIR/ci-ret-summary-fail.log"
    mkdir -p "$CI_RET_SUM_FAIL_DIR"
    set +e
    (cd "$CI_RET_SUM_FAIL_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_RET_SCRIPT" --mode failure --summary > "$CI_RET_SUM_FAIL_LOG" 2>&1)
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres =
        if (project.modules.postgres.enable or false) then
          import ./nixfied/.framework/postgres { inherit pkgs project slots; }
        else
          null;
      nginx =
        if (project.modules.nginx.enable or false) then
          import ./nixfied/.framework/nginx { inherit pkgs project slots; }
        else
          null;
      minio =
        if (project.modules.minio.enable or false) then
          import ./nixfied/.framework/minio { inherit pkgs project slots; }
        else
          null;
      reth =
        if (project.modules.reth.enable or false) then
          import ./nixfied/.framework/reth { inherit pkgs project slots; }
        else
          null;
      helios =
        if (project.modules.helios.enable or false) then
          import ./nixfied/.framework/helios { inherit pkgs project slots; }
        else
          null;
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots postgres nginx minio reth helios; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
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
        POSTGRES_INIT POSTGRES_START POSTGRES_STOP POSTGRES_HEALTH POSTGRES_READY POSTGRES_SETUP_DB POSTGRES_FULL_START POSTGRES_FULL_START_TEST \
        NGINX_INIT NGINX_START NGINX_STOP NGINX_HEALTH NGINX_READY NGINX_SITE_PROXY NGINX_SITE_STATIC \
        MINIO_INIT MINIO_START MINIO_STOP MINIO_HEALTH MINIO_READY MINIO_CHECK_CONFIG MINIO_BUCKET_LIST \
        RETH_INIT RETH_START RETH_STOP RETH_HEALTH RETH_READY RETH_CHECK_CONFIG \
        HELIOS_INIT HELIOS_START HELIOS_STOP HELIOS_HEALTH HELIOS_READY HELIOS_CHECK_CONFIG
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
      print_log_tail "$DEV_LOG" 50
      echo "" >&2
      echo "Matches for 'name}' in script:" >&2
      grep -n "name}" "$DEV_SCRIPT" >&2 || true
      exit "$DEV_RC"
    fi

    log "fixtures services prelude"
    FIX_SVC_DIR="$WORKDIR/fixtures-services"
    mkdir -p "$FIX_SVC_DIR"
    FIX_SVC_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres =
        if (project.modules.postgres.enable or false) then
          import ./nixfied/.framework/postgres { inherit pkgs project slots; }
        else
          null;
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres;
        nginx = null;
        minio = null;
        reth = null;
        helios = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };

      fixtures = {
        artifacts = {
          prefix = "fx";
          logs = true;
        };
        services = [
          {
            name = "postgres";
            profile = "test";
            timeout = 120;
            interval = 1;
            logs = true;
          }
        ];
      };

      fixturePrelude = lib.fixtures.renderPrelude {
        inherit fixtures;
        contextName = "fixtures-services";
        defaultProfile = "default";
        defaultLogs = true;
      };
    in
    lib.mkAppScript {
      name = "fixtures-services";
      env = {
        "''${project.project.envVar}" = "test";
        "''${project.project.slotVar}" = "0";
      };
      useDeps = false;
      script = '''
        fail() {
          echo "FAIL: $*" >&2
          exit 1
        }

        pick_port() {
          local port
          local i
          for i in $(seq 1 40); do
            port=$(( (RANDOM % 20000) + 20000 ))
            if ! nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
              echo "$port"
              return 0
            fi
          done
          return 1
        }

        export PGPORT="$(pick_port)" || fail "failed to pick port"
        export CI_ARTIFACTS_DIR="$PWD/.artifacts"

        ''${fixturePrelude}

        LOG="$CI_ARTIFACTS_DIR/fx-0-postgres.log"
        [ -f "$LOG" ] || fail "missing fixture service log: $LOG"
        for i in $(seq 1 200); do
          if grep -q "OK: Database 'app_test' ready" "$LOG"; then
            exit 0
          fi
          sleep 0.1
        done
        fail "expected test db setup in log"
      ''';
    }
    NIX
    )

    FIX_SVC_SCRIPT=$(build_expr "$FIX_SVC_EXPR")
    FIX_SVC_LOG="$WORKDIR/fixtures-services.log"
    set +e
    (cd "$FIX_SVC_DIR" && "$FIX_SVC_SCRIPT" >"$FIX_SVC_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      echo "Fixtures services fixture failed (rc=$RC)." >&2
      echo "" >&2
      echo "Fixture output (last 100 lines):" >&2
      print_log_tail "$FIX_SVC_LOG" 100
      exit "$RC"
    fi
    assert_contains "$FIX_SVC_LOG" "INFO: fixture service start name=postgres profile=test"
    assert_contains "$FIX_SVC_LOG" "OK: fixture service ready service=postgres profile=test"

    log "fixture_start_service"
    FIX_START_DIR="$WORKDIR/fixture-start-service"
    mkdir -p "$FIX_START_DIR"
    FIX_START_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres =
        if (project.modules.postgres.enable or false) then
          import ./nixfied/.framework/postgres { inherit pkgs project slots; }
        else
          null;
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres;
        nginx = null;
        minio = null;
        reth = null;
        helios = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
    lib.mkAppScript {
      name = "fixture-start-service";
      env = {
        "''${project.project.envVar}" = "test";
        "''${project.project.slotVar}" = "0";
      };
      useDeps = false;
      script = '''
        fail() {
          echo "FAIL: $*" >&2
          exit 1
        }

        pick_port() {
          local port
          local i
          for i in $(seq 1 40); do
            port=$(( (RANDOM % 20000) + 20000 ))
            if ! nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
              echo "$port"
              return 0
            fi
          done
          return 1
        }

        export PGPORT="$(pick_port)" || fail "failed to pick port"
        export CI_ARTIFACTS_DIR="$PWD/.artifacts"

        LOGFILE="$(artifact_path "postgres-fixture.log")"
        fixture_start_service postgres test 120 1 "$LOGFILE"
        [ -f "$LOGFILE" ] || fail "missing fixture_start_service log: $LOGFILE"
        for i in $(seq 1 200); do
          if grep -q "OK: Database 'app_test' ready" "$LOGFILE"; then
            break
          fi
          sleep 0.1
        done
        grep -q "OK: Database 'app_test' ready" "$LOGFILE" || fail "expected test db setup in log"

        # Explicitly run cleanups so we can assert the port closes.
        _run_cleanups

        for i in $(seq 1 30); do
          if nc -z 127.0.0.1 "$PGPORT" >/dev/null 2>&1; then
            sleep 0.2
            continue
          fi
          echo "OK: fixture cleanup stopped postgres port=$PGPORT"
          exit 0
        done
        fail "postgres still listening after cleanup port=$PGPORT"
      ''';
    }
    NIX
    )

    FIX_START_SCRIPT=$(build_expr "$FIX_START_EXPR")
    FIX_START_LOG="$WORKDIR/fixture-start-service.log"
    set +e
    (cd "$FIX_START_DIR" && "$FIX_START_SCRIPT" >"$FIX_START_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      echo "fixture_start_service fixture failed (rc=$RC)." >&2
      echo "" >&2
      echo "Fixture output (last 100 lines):" >&2
      print_log_tail "$FIX_START_LOG" 100
      exit "$RC"
    fi
    assert_contains "$FIX_START_LOG" "OK: fixture service ready service=postgres profile=test"
    assert_contains "$FIX_START_LOG" "OK: fixture cleanup stopped postgres"

    log "fixture_start_service readiness"
    FIX_START_READY_DIR="$WORKDIR/fixture-start-service-readiness"
    mkdir -p "$FIX_START_READY_DIR"
    FIX_START_READY_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres =
        if (project.modules.postgres.enable or false) then
          import ./nixfied/.framework/postgres { inherit pkgs project slots; }
        else
          null;
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres;
        nginx = null;
        minio = null;
        reth = null;
        helios = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
    lib.mkAppScript {
      name = "fixture-start-service-readiness";
      env = {
        "''${project.project.envVar}" = "test";
        "''${project.project.slotVar}" = "0";
      };
      useDeps = false;
      script = '''
        fail() {
          echo "FAIL: $*" >&2
          exit 1
        }

        pick_port() {
          local port
          local i
          for i in $(seq 1 40); do
            port=$(( (RANDOM % 20000) + 20000 ))
            if ! nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
              echo "$port"
              return 0
            fi
          done
          return 1
        }

        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'while true; do' '  sleep 1' 'done' > "$PWD/mock-start.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 1' > "$PWD/mock-health.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'COUNT_FILE="$MOCK_READY_COUNT_FILE"' 'if [ -z "$COUNT_FILE" ]; then COUNT_FILE="$PWD/mock-ready.count"; fi' 'count=0' 'if [ -f "$COUNT_FILE" ]; then count=$(cat "$COUNT_FILE"); fi' 'count=$((count + 1))' 'echo "$count" > "$COUNT_FILE"' 'if [ "$count" -lt 3 ]; then exit 1; fi' 'exit 0' > "$PWD/mock-ready.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 0' > "$PWD/mock-status.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 0' > "$PWD/mock-stop.sh"
        chmod +x "$PWD/mock-start.sh" "$PWD/mock-health.sh" "$PWD/mock-ready.sh" "$PWD/mock-status.sh" "$PWD/mock-stop.sh"

        export MOCK_READY_COUNT_FILE="$PWD/mock-ready.count"
        export MOCKSVC_START="$PWD/mock-start.sh"
        export MOCKSVC_HEALTH="$PWD/mock-health.sh"
        export MOCKSVC_READY="$PWD/mock-ready.sh"
        export MOCKSVC_STATUS="$PWD/mock-status.sh"
        export MOCKSVC_STOP="$PWD/mock-stop.sh"

        fixture_start_service mocksvc default 5 1
        READY_ATTEMPTS="$(cat "$MOCK_READY_COUNT_FILE" 2>/dev/null || echo 0)"
        [ "$READY_ATTEMPTS" -ge 3 ] || fail "fixture_start_service should retry READY hook attempts=$READY_ATTEMPTS"
        _run_cleanups
        _cleanup_actions=()
        _cleanup_initialized=false
        echo "OK: fixture_start_service retried READY hook"

        export PGPORT="$(pick_port)" || fail "failed to pick port"
        export CI_ARTIFACTS_DIR="$PWD/.artifacts"
        LOGFILE="$(artifact_path "postgres-keep-running.log")"

        fixture_start_service postgres test 120 1 "$LOGFILE" 1
        _run_cleanups

        PORT_UP=0
        for i in $(seq 1 30); do
          if nc -z 127.0.0.1 "$PGPORT" >/dev/null 2>&1; then
            PORT_UP=1
            break
          fi
          sleep 0.2
        done
        [ "$PORT_UP" -eq 1 ] || fail "postgres should still be running with keep_running=1 port=$PGPORT"

        run_hook POSTGRES_STOP
        PORT_DOWN=0
        for i in $(seq 1 30); do
          if nc -z 127.0.0.1 "$PGPORT" >/dev/null 2>&1; then
            sleep 0.2
          else
            PORT_DOWN=1
            break
          fi
        done
        [ "$PORT_DOWN" -eq 1 ] || fail "postgres should stop after explicit POSTGRES_STOP port=$PGPORT"
        echo "OK: fixture_start_service keep_running preserved service"

        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'echo "$$" > "$PWD/mock-timeout.pid"' 'while true; do' '  if [ -n "$MOCK_TIMEOUT_LOGFILE" ]; then rm -f "$MOCK_TIMEOUT_LOGFILE"; fi' '  sleep 0.1' 'done' > "$PWD/mock-timeout-start.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 1' > "$PWD/mock-timeout-health.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 1' > "$PWD/mock-timeout-ready.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'exit 1' > "$PWD/mock-timeout-status.sh"
        printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'if [ -f "$PWD/mock-timeout.pid" ]; then pid=$(cat "$PWD/mock-timeout.pid" 2>/dev/null || true); if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; fi; fi; exit 0' > "$PWD/mock-timeout-stop.sh"
        chmod +x "$PWD/mock-timeout-start.sh" "$PWD/mock-timeout-health.sh" "$PWD/mock-timeout-ready.sh" "$PWD/mock-timeout-status.sh" "$PWD/mock-timeout-stop.sh"

        export TIMEOUTSVC_START="$PWD/mock-timeout-start.sh"
        export TIMEOUTSVC_HEALTH="$PWD/mock-timeout-health.sh"
        export TIMEOUTSVC_READY="$PWD/mock-timeout-ready.sh"
        export TIMEOUTSVC_STATUS="$PWD/mock-timeout-status.sh"
        export TIMEOUTSVC_STOP="$PWD/mock-timeout-stop.sh"

        TIMEOUT_LOG="$PWD/missing-timeout.log"
        export MOCK_TIMEOUT_LOGFILE="$TIMEOUT_LOG"
        set +e
        fixture_start_service timeoutsvc default 1 0.1 "$TIMEOUT_LOG"
        TIMEOUT_RC=$?
        set -e
        [ "$TIMEOUT_RC" -ne 0 ] || fail "expected timeoutsvc readiness failure"
        [ ! -f "$TIMEOUT_LOG" ] || fail "timeoutsvc log should be absent path=$TIMEOUT_LOG"
        TIMEOUT_PID="$(cat "$PWD/mock-timeout.pid" 2>/dev/null || true)"
        if [ -n "$TIMEOUT_PID" ] && kill -0 "$TIMEOUT_PID" 2>/dev/null; then
          fail "timeoutsvc process still running pid=$TIMEOUT_PID"
        fi
        _run_cleanups
        _cleanup_actions=()
        _cleanup_initialized=false
        echo "OK: fixture_start_service timeout cleanup removed process"
      ''';
    }
    NIX
    )

    FIX_START_READY_SCRIPT=$(build_expr "$FIX_START_READY_EXPR")
    FIX_START_READY_LOG="$WORKDIR/fixture-start-service-readiness.log"
    set +e
    (cd "$FIX_START_READY_DIR" && "$FIX_START_READY_SCRIPT" >"$FIX_START_READY_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      echo "fixture_start_service readiness fixture failed (rc=$RC)." >&2
      echo "" >&2
      echo "Fixture output (last 120 lines):" >&2
      print_log_tail "$FIX_START_READY_LOG" 120
      exit "$RC"
    fi
    assert_contains "$FIX_START_READY_LOG" "OK: fixture_start_service retried READY hook"
    assert_contains "$FIX_START_READY_LOG" "OK: fixture_start_service keep_running preserved service"
    assert_contains "$FIX_START_READY_LOG" "WARN: fixture log file missing path="
    assert_contains "$FIX_START_READY_LOG" "OK: fixture_start_service timeout cleanup removed process"
    assert_not_contains "$FIX_START_READY_LOG" "tail: cannot open"

    log "fixtures env resolvers"
    FIX_ENV_DIR="$WORKDIR/fixtures-env"
    mkdir -p "$FIX_ENV_DIR"
    FIX_ENV_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; minio = null; reth = null; helios = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
    lib.mkAppScript {
      name = "fixtures-env";
      env = {
        POSTGRES_PORT = "15432";
        RETHHTTP_PORT = "18545";
        HELIOSRPC_PORT = "18547";
        MINIOAPI_PORT = "19000";
        MINIO_BUCKET = "bucket1";
        MINIO_REGION = "us-west-2";
        MINIO_PREFIX = "pfx";
        SOURCE_VAR = "from-env";
      };
      fixtures = {
        env = {
          FIX_POSTGRES_URL = { from = "postgres.url"; database = "db1"; };
          FIX_RETH_HTTP_URL = { from = "reth.httpUrl"; };
          FIX_HELIOS_RPC_URL = { from = "helios.rpcUrl"; };
          FIX_MINIO_ENDPOINT = { from = "minio.endpoint"; };
          FIX_MINIO_BUCKET = { from = "minio.bucket"; };
          FIX_MINIO_REGION = { from = "minio.region"; };
          FIX_MINIO_PREFIX = { from = "minio.prefix"; };
          FIX_FROM_ENV = { from = "env"; var = "SOURCE_VAR"; };
        };
      };
      useDeps = false;
      script = '''
        fail() {
          echo "FAIL: $*" >&2
          exit 1
        }

        [ "$FIX_POSTGRES_URL" = "postgresql://postgres:postgres@127.0.0.1:15432/db1" ] || fail "FIX_POSTGRES_URL mismatch: $FIX_POSTGRES_URL"
        [ "$FIX_RETH_HTTP_URL" = "http://127.0.0.1:18545" ] || fail "FIX_RETH_HTTP_URL mismatch: $FIX_RETH_HTTP_URL"
        [ "$FIX_HELIOS_RPC_URL" = "http://127.0.0.1:18547" ] || fail "FIX_HELIOS_RPC_URL mismatch: $FIX_HELIOS_RPC_URL"
        [ "$FIX_MINIO_ENDPOINT" = "http://127.0.0.1:19000" ] || fail "FIX_MINIO_ENDPOINT mismatch: $FIX_MINIO_ENDPOINT"
        [ "$FIX_MINIO_BUCKET" = "bucket1" ] || fail "FIX_MINIO_BUCKET mismatch: $FIX_MINIO_BUCKET"
        [ "$FIX_MINIO_REGION" = "us-west-2" ] || fail "FIX_MINIO_REGION mismatch: $FIX_MINIO_REGION"
        [ "$FIX_MINIO_PREFIX" = "pfx" ] || fail "FIX_MINIO_PREFIX mismatch: $FIX_MINIO_PREFIX"
        [ "$FIX_FROM_ENV" = "from-env" ] || fail "FIX_FROM_ENV mismatch: $FIX_FROM_ENV"
      ''';
    }
    NIX
    )

    FIX_ENV_SCRIPT=$(build_expr "$FIX_ENV_EXPR")
    FIX_ENV_LOG="$WORKDIR/fixtures-env.log"
    set +e
    (cd "$FIX_ENV_DIR" && "$FIX_ENV_SCRIPT" >"$FIX_ENV_LOG" 2>&1)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      echo "Fixtures env resolvers fixture failed (rc=$RC)." >&2
      echo "" >&2
      echo "Fixture output (last 100 lines):" >&2
      print_log_tail "$FIX_ENV_LOG" 100
      exit "$RC"
    fi

    log "fixtures service validation"
    FIX_BAD_UNKNOWN_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; minio = null; reth = null; helios = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      pkgs.writeText "fixture-prelude" (lib.fixtures.renderPrelude {
        fixtures = { services = [ "does-not-exist" ]; };
        contextName = "bad-fixtures";
        defaultProfile = "default";
        defaultLogs = true;
      })
    NIX
    )

    FIX_BAD_UNKNOWN_LOG="$WORKDIR/fixtures-bad-unknown.log"
    set +e
    build_expr "$FIX_BAD_UNKNOWN_EXPR" >"$FIX_BAD_UNKNOWN_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected unknown fixture service evaluation to fail"
    fi
    assert_contains "$FIX_BAD_UNKNOWN_LOG" "Unknown fixture service"

    FIX_BAD_DISABLED_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base conf // {
        modules = conf.modules // {
          postgres = (conf.modules.postgres or { }) // { enable = false; };
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; minio = null; reth = null; helios = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      pkgs.writeText "fixture-prelude" (lib.fixtures.renderPrelude {
        fixtures = { services = [ "postgres" ]; };
        contextName = "bad-fixtures";
        defaultProfile = "default";
        defaultLogs = true;
      })
    NIX
    )

    FIX_BAD_DISABLED_LOG="$WORKDIR/fixtures-bad-disabled.log"
    set +e
    build_expr "$FIX_BAD_DISABLED_EXPR" >"$FIX_BAD_DISABLED_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected disabled fixture service evaluation to fail"
    fi
    assert_contains "$FIX_BAD_DISABLED_LOG" "requires project.modules.postgres.enable = true"

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
              command = "echo $KEEP_ME";
              workingDir = ".";
              readiness = {
                type = "exec";
                command = "test -n \"$KEEP_ME\"";
                initialDelaySeconds = 1;
                periodSeconds = 1;
                timeoutSeconds = 1;
                failureThreshold = 1;
              };
            };
          };
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
    in
      supervisor.generateConfig
    NIX
    )

    SUP_SCRIPT=$(build_expr "$SUP_EXPR")
    SUP_CONFIG=$(PROJECT_ENV=dev NIX_ENV=0 "$SUP_SCRIPT")
    assert_file_exists "$SUP_CONFIG"
    assert_contains "$SUP_CONFIG" "processes:"
    assert_contains "$SUP_CONFIG" "app:"
    assert_contains "$SUP_CONFIG" 'echo $KEEP_ME'

    log "supervisor config missing readiness"
    SUP_BAD_EXPR=$(cat <<'NIX'
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
              command = "echo missing readiness";
              workingDir = ".";
            };
          };
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
    in
      supervisor.generateConfig
    NIX
    )

    SUP_BAD_LOG="$WORKDIR/supervisor-config-missing-readiness.log"
    set +e
    build_expr "$SUP_BAD_EXPR" >"$SUP_BAD_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected supervisor config evaluation to fail when readiness is missing"
    fi
    assert_contains "$SUP_BAD_LOG" "Missing readiness probe for services"

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
    if [ ! -d "$INSTALL_TARGET/nixfied/.framework" ]; then
      fail "expected nixfied/.framework directory in installer target"
    fi
    assert_file_absent "$INSTALL_TARGET/nixfied/.framework/.workspace"
    assert_file_absent "$INSTALL_TARGET/NIXFIED_PROMPT_PLAN.md"
    assert_file_exists "$INSTALL_TARGET/nixfied/local/default.nix"
    assert_file_exists "$INSTALL_TARGET/nixfied/README.md"
    assert_file_exists "$INSTALL_TARGET/nixfied/VENDORED.txt"
    assert_contains "$INSTALL_TARGET/nixfied/VENDORED.txt" "Framework source revision"

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
    if [ ! -d "$INSTALL_WT_TARGET/nixfied/.framework" ]; then
      fail "expected nixfied/.framework directory in worktree target"
    fi
    assert_file_absent "$INSTALL_WT_TARGET/nixfied/.framework/.workspace"
    assert_file_exists "$INSTALL_WT_TARGET/nixfied/local/default.nix"
    assert_file_exists "$INSTALL_WT_TARGET/nixfied/README.md"

    log "example project apps (basic install)"
    BASIC_HELP="$WORKDIR/basic-help.txt"
    run_app "$INSTALL_TARGET" help > "$BASIC_HELP"
    assert_contains "$BASIC_HELP" "Commands:"
    assert_contains "$BASIC_HELP" "dev  Start the dev workflow"
    assert_contains "$BASIC_HELP" "test  Run tests"
    assert_contains "$BASIC_HELP" "build  Build artifacts"
    assert_contains "$BASIC_HELP" "check  Run quality checks"
    assert_contains "$BASIC_HELP" "format  Format Nix files"
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
    PROJECT_ENV=dev NIXFIED_ENV=1 run_app "$INSTALL_TARGET" ports > "$PORTS_ALIAS_OUT"
    assert_contains "$PORTS_ALIAS_OUT" "Port assignments for slot 1, env dev"
    PORTS_NIX_ENV_OUT="$WORKDIR/ports-nix-env.txt"
    PROJECT_ENV=dev NIX_ENV=1 run_app "$INSTALL_TARGET" ports > "$PORTS_NIX_ENV_OUT"
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

    log "service api contract enforcement"
    BAD_SERVICE_API_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      serviceApi = import ./nixfied/.framework/lib/service-api.nix { inherit pkgs; };
    in
      pkgs.writeText "service-api-validated" (builtins.toJSON (serviceApi.validateServiceApis {
        postgres = {
          version = 1;
          service = "postgres";
          summary = "bad contract";
          details = "missing required core ops and ci profile";
          profiles = [ "dev" "prod" "test" ];
          coreOps = {
            init = {
              script = "/bin/true";
              summary = "init";
              details = "init";
            };
          };
          artifacts = { };
        };
      }))
    NIX
    )
    BAD_SERVICE_API_LOG="$WORKDIR/bad-service-api-contract.log"
    set +e
    build_expr "$BAD_SERVICE_API_EXPR" > "$BAD_SERVICE_API_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected service API contract violation to fail"
    fi
    assert_contains "$BAD_SERVICE_API_LOG" "Nixfied service API contract violated"
    assert_contains "$BAD_SERVICE_API_LOG" "publicApi.profiles missing required values: ci"
    assert_contains "$BAD_SERVICE_API_LOG" "publicApi.coreOps missing required ops"

    log "installer upgrade preserves project"
    echo "# NIXFIED_UPGRADE_TEST_MARKER" >> "$INSTALL_TARGET/nixfied/project/conf.nix"
    echo "# NIXFIED_LOCAL_UPGRADE_TEST_MARKER" >> "$INSTALL_TARGET/nixfied/local/default.nix"
    (cd "$INSTALL_TARGET" && nix run "path:$ROOT"#framework::upgrade -- --force >/dev/null)
    assert_contains "$INSTALL_TARGET/nixfied/project/conf.nix" "NIXFIED_UPGRADE_TEST_MARKER"
    assert_contains "$INSTALL_TARGET/nixfied/local/default.nix" "NIXFIED_LOCAL_UPGRADE_TEST_MARKER"
    assert_file_absent "$INSTALL_TARGET/nixfied/.framework/.workspace"
    assert_file_exists "$INSTALL_TARGET/nixfied/README.md"
    assert_contains "$INSTALL_TARGET/nixfied/VENDORED.txt" "Framework source revision"

    log "framework marker toggle"
    mkdir -p "$INSTALL_TARGET/nixfied/.framework"
    touch "$INSTALL_TARGET/nixfied/.framework/.workspace"
    FRAMEWORK_HELP="$WORKDIR/framework-help.txt"
    run_app "$INSTALL_TARGET" "framework::prompt-plan" --help > "$FRAMEWORK_HELP"
    assert_contains "$FRAMEWORK_HELP" "prompt-plan"
    FRAMEWORK_INSTALL_HELP="$WORKDIR/framework-install-help.txt"
    run_app "$INSTALL_TARGET" "framework::install" --help > "$FRAMEWORK_INSTALL_HELP"
    assert_contains "$FRAMEWORK_INSTALL_HELP" "framework::install"
    FRAMEWORK_UPGRADE_HELP="$WORKDIR/framework-upgrade-help.txt"
    run_app "$INSTALL_TARGET" "framework::upgrade" --help > "$FRAMEWORK_UPGRADE_HELP"
    assert_contains "$FRAMEWORK_UPGRADE_HELP" "framework::upgrade"
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
    rm -f "$INSTALL_TARGET/nixfied/.framework/.workspace"

    log "installer re-entry"
    REENTRY_BASE="$WORKDIR/install-reentry"
    init_repo "$REENTRY_BASE"
    (cd "$REENTRY_BASE" && nix run "path:$ROOT"#framework::install >/dev/null)
    REENTRY_TARGET="$REENTRY_BASE"
    assert_file_exists "$REENTRY_TARGET/flake.nix"
    assert_file_absent "$REENTRY_TARGET/nixfied/.framework/.workspace"
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
    assert_file_absent "$FILTER_TARGET/nixfied/.framework/.workspace"
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
    assert_app_missing "$FILTER_TARGET" "format"
    run_app_quiet "$FILTER_TARGET" ci --summary

    log "installer filter build alias"
    INSTALL_FILTER_BUILD="$WORKDIR/install-filter-build"
    init_repo "$INSTALL_FILTER_BUILD"
    (cd "$INSTALL_FILTER_BUILD" && nix run "path:$ROOT"#framework::install -- --filter=conf,build >/dev/null)
    FILTER_BUILD_TARGET="$INSTALL_FILTER_BUILD"
    assert_file_exists "$FILTER_BUILD_TARGET/nixfied/project/prod.nix"
    assert_file_absent "$FILTER_BUILD_TARGET/nixfied/project/dev.nix"
    assert_file_absent "$FILTER_BUILD_TARGET/nixfied/project/test.nix"
    assert_file_absent "$FILTER_BUILD_TARGET/nixfied/project/quality.nix"
    assert_file_absent "$FILTER_BUILD_TARGET/nixfied/project/ci.nix"
    if ! grep -q "prod.nix" "$FILTER_BUILD_TARGET/nixfied/project/default.nix"; then
      fail "default.nix should include prod.nix when filtered with build alias"
    fi
    FILTER_BUILD_HELP="$WORKDIR/filter-build-help.txt"
    run_app "$FILTER_BUILD_TARGET" help > "$FILTER_BUILD_HELP"
    assert_contains "$FILTER_BUILD_HELP" "build  Build artifacts"
    assert_app_missing "$FILTER_BUILD_TARGET" "ci"
    assert_app_missing "$FILTER_BUILD_TARGET" "check"
    assert_app_missing "$FILTER_BUILD_TARGET" "format"
    run_app_quiet "$FILTER_BUILD_TARGET" build

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
    FORCE_BRANCH_BEFORE=$(git -C "$INSTALL_FORCE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    (cd "$INSTALL_FORCE" && nix run "path:$ROOT"#framework::install -- --force >/dev/null)
    FORCE_BRANCH_AFTER=$(git -C "$INSTALL_FORCE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    if [ "$FORCE_BRANCH_AFTER" != "$FORCE_BRANCH_BEFORE" ]; then
      fail "expected --force install to keep branch '$FORCE_BRANCH_BEFORE' (got: $FORCE_BRANCH_AFTER)"
    fi
    assert_file_exists "$INSTALL_FORCE/flake.nix"
    if [ ! -d "$INSTALL_FORCE/nixfied" ]; then
      fail "expected nixfied/ directory in force target"
    fi
    assert_file_absent "$INSTALL_FORCE/nixfied/.framework/.workspace"
    assert_file_absent "$INSTALL_FORCE/NIXFIED_PROMPT_PLAN.md"

    log "upgrade force keeps current branch"
    echo "# NIXFIED_FORCE_UPGRADE_TEST_MARKER" >> "$INSTALL_FORCE/nixfied/project/conf.nix"
    FORCE_UPGRADE_BRANCH_BEFORE=$(git -C "$INSTALL_FORCE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    (cd "$INSTALL_FORCE" && nix run "path:$ROOT"#framework::upgrade -- --force >/dev/null)
    FORCE_UPGRADE_BRANCH_AFTER=$(git -C "$INSTALL_FORCE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)
    if [ "$FORCE_UPGRADE_BRANCH_AFTER" != "$FORCE_UPGRADE_BRANCH_BEFORE" ]; then
      fail "expected --force upgrade to keep branch '$FORCE_UPGRADE_BRANCH_BEFORE' (got: $FORCE_UPGRADE_BRANCH_AFTER)"
    fi
    assert_contains "$INSTALL_FORCE/nixfied/project/conf.nix" "NIXFIED_FORCE_UPGRADE_TEST_MARKER"

    log "example project apps (force install)"
    FORCE_HELP="$WORKDIR/force-help.txt"
    run_app "$INSTALL_FORCE" help > "$FORCE_HELP"
    assert_contains "$FORCE_HELP" "Commands:"
    assert_contains "$FORCE_HELP" "dev  Start the dev workflow"
    assert_contains "$FORCE_HELP" "test  Run tests"
    assert_contains "$FORCE_HELP" "build  Build artifacts"
    assert_contains "$FORCE_HELP" "check  Run quality checks"
    assert_contains "$FORCE_HELP" "format  Format Nix files"
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
      ephemeral = import ./nixfied/.framework/ephemeral.nix { inherit pkgs project; };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
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
      print_log_tail "$EPHEM_LOG" 50
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
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
      print_log_tail "$REG_LOG" 50
      exit "$REG_RC"
    fi

    log "postgres extensions"
    PG_EXT_DIR="$WORKDIR/postgres-extensions"
    mkdir -p "$PG_EXT_DIR"
    PG_EXT_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules.postgres.enable = true;
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gnugrep
            pkgs.gzip
            pkgs.lsof
            pkgs.netcat
            pkgs.python3
          ];
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres = import ./nixfied/.framework/postgres { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres;
        nginx = null;
        minio = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "postgres-extensions-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "0";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/postgres/extensions.nix {
          inherit (postgres)
            backup
            listBackups
            verifyBackup
            cleanupBackups
            restore
            findBackupForCommit
            testRollback
            testMigrations
            ensureMigrationTested
            markMigrationTested
            detectDrift
            checkPort
            killPort
            ;
        };
      }
    NIX
    )

    PG_EXT_SCRIPT=$(build_expr "$PG_EXT_EXPR")
    PG_EXT_LOG="$WORKDIR/postgres-extensions.log"
    set +e
    (cd "$PG_EXT_DIR" && "$PG_EXT_SCRIPT" >"$PG_EXT_LOG" 2>&1)
    PG_EXT_RC=$?
    set -e
    if [ "$PG_EXT_RC" -ne 0 ]; then
      echo "Postgres extensions fixture failed (rc=$PG_EXT_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$PG_EXT_LOG" 80
      exit "$PG_EXT_RC"
    fi

    log "nginx site lifecycle"
    NGX_DIR="$WORKDIR/nginx-site-lifecycle"
    mkdir -p "$NGX_DIR"
    NGX_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules.nginx.enable = true;
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gnugrep
            pkgs.netcat
          ];
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      nginx = import ./nixfied/.framework/nginx { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots nginx;
        postgres = null;
        minio = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "nginx-site-lifecycle-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "0";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/nginx/site-lifecycle.nix {
          nginxInit = toString nginx.init;
          nginxStart = toString nginx.start;
          nginxStop = toString nginx.stop;
          nginxReload = toString nginx.reload;
          nginxCheckConfig = toString nginx.checkConfig;
          nginxStatus = toString nginx.status;
          nginxHealth = toString nginx.health;
          nginxReady = toString nginx.ready;
          nginxSiteAdd = toString nginx.addSite;
          nginxSiteStatic = toString nginx.writeStaticSite;
          nginxSiteList = toString nginx.listSites;
          nginxSiteDisable = toString nginx.disableSite;
          nginxSiteEnable = toString nginx.enableSite;
          nginxSiteRemove = toString nginx.removeSite;
        };
      }
    NIX
    )

    NGX_SCRIPT=$(build_expr "$NGX_EXPR")
    NGX_LOG="$WORKDIR/nginx-site-lifecycle.log"
    set +e
    (cd "$NGX_DIR" && "$NGX_SCRIPT" >"$NGX_LOG" 2>&1)
    NGX_RC=$?
    set -e
    if [ "$NGX_RC" -ne 0 ]; then
      echo "Nginx site lifecycle fixture failed (rc=$NGX_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$NGX_LOG" 80
      exit "$NGX_RC"
    fi

    log "minio bucket ops"
    MINIO_FIX_DIR="$WORKDIR/minio-bucket-ops"
    mkdir -p "$MINIO_FIX_DIR"
    MINIO_FIX_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules.minio.enable = true;
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gnugrep
            pkgs.netcat
          ];
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      minio = import ./nixfied/.framework/minio { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots minio;
        postgres = null;
        nginx = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "minio-bucket-ops-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "0";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/minio/bucket-ops.nix {
          minioInit = toString minio.init;
          minioStart = toString minio.start;
          minioStop = toString minio.stop;
          minioHealth = toString minio.health;
          minioReady = toString minio.ready;
          minioStatus = toString minio.status;
          minioCheckConfig = toString minio.checkConfig;
          minioBucketCreate = toString minio.bucketCreate;
          minioBucketDelete = toString minio.bucketDelete;
          minioBucketList = toString minio.bucketList;
          minioPolicyApply = toString minio.policyApply;
        };
      }
    NIX
    )

    MINIO_FIX_SCRIPT=$(build_expr "$MINIO_FIX_EXPR")
    MINIO_FIX_LOG="$WORKDIR/minio-bucket-ops.log"
    set +e
    (cd "$MINIO_FIX_DIR" && "$MINIO_FIX_SCRIPT" >"$MINIO_FIX_LOG" 2>&1)
    MINIO_FIX_RC=$?
    set -e
    if [ "$MINIO_FIX_RC" -ne 0 ]; then
      echo "MinIO bucket ops fixture failed (rc=$MINIO_FIX_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$MINIO_FIX_LOG" 80
      exit "$MINIO_FIX_RC"
    fi

    log "reth lifecycle"
    RETH_FIX_DIR="$WORKDIR/reth-lifecycle"
    mkdir -p "$RETH_FIX_DIR"
    RETH_FIX_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate (pkgs.lib.recursiveUpdate base conf) {
        modules = {
          postgres.enable = false;
          nginx.enable = false;
          minio.enable = false;
          helios.enable = false;
          reth.enable = true;
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      reth = import ./nixfied/.framework/reth { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots reth;
        postgres = null;
        nginx = null;
        minio = null;
        helios = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "reth-lifecycle-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "8";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/reth/lifecycle.nix {
          rethInit = toString reth.init;
          rethStart = toString reth.start;
          rethStop = toString reth.stop;
          rethHealth = toString reth.health;
          rethReady = toString reth.ready;
          rethStatus = toString reth.status;
          rethCheckConfig = toString reth.checkConfig;
        };
      }
    NIX
    )

    RETH_FIX_SCRIPT=$(build_expr "$RETH_FIX_EXPR")
    RETH_FIX_LOG="$WORKDIR/reth-lifecycle.log"
    set +e
    (cd "$RETH_FIX_DIR" && "$RETH_FIX_SCRIPT" >"$RETH_FIX_LOG" 2>&1)
    RETH_FIX_RC=$?
    set -e
    if [ "$RETH_FIX_RC" -ne 0 ]; then
      echo "Reth lifecycle fixture failed (rc=$RETH_FIX_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$RETH_FIX_LOG" 80
      exit "$RETH_FIX_RC"
    fi

    log "helios lifecycle"
    HELIOS_FIX_DIR="$WORKDIR/helios-lifecycle"
    mkdir -p "$HELIOS_FIX_DIR"
    HELIOS_FIX_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      conf = import ./tests/framework/fixtures/modules/conf.nix { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate (pkgs.lib.recursiveUpdate base conf) {
        modules = {
          postgres.enable = false;
          nginx.enable = false;
          minio.enable = false;
          reth.enable = true;
          helios.enable = true;
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      reth = import ./nixfied/.framework/reth { inherit pkgs project slots; };
      helios = import ./nixfied/.framework/helios { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots reth helios;
        postgres = null;
        nginx = null;
        minio = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "helios-lifecycle-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "9";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/helios/lifecycle.nix {
          rethBin = "''${reth.reth}/bin/reth";
          heliosBin = "''${helios.helios}/bin/helios";
          rethInit = toString reth.init;
          rethStart = toString reth.start;
          rethStop = toString reth.stop;
          rethHealth = toString reth.health;
          heliosInit = toString helios.init;
          heliosStart = toString helios.start;
          heliosStop = toString helios.stop;
          heliosHealth = toString helios.health;
          heliosReady = toString helios.ready;
          heliosStatus = toString helios.status;
          heliosCheckConfig = toString helios.checkConfig;
        };
      }
    NIX
    )

    HELIOS_FIX_SCRIPT=$(build_expr "$HELIOS_FIX_EXPR")
    HELIOS_FIX_LOG="$WORKDIR/helios-lifecycle.log"
    set +e
    (cd "$HELIOS_FIX_DIR" && "$HELIOS_FIX_SCRIPT" >"$HELIOS_FIX_LOG" 2>&1)
    HELIOS_FIX_RC=$?
    set -e
    if [ "$HELIOS_FIX_RC" -ne 0 ]; then
      echo "Helios lifecycle fixture failed (rc=$HELIOS_FIX_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$HELIOS_FIX_LOG" 80
      exit "$HELIOS_FIX_RC"
    fi

    log "supervisor management"
    SUP_MGMT_DIR="$WORKDIR/supervisor-management"
    mkdir -p "$SUP_MGMT_DIR"
    SUP_MGMT_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.gzip
          ];
        supervisor = {
          enable = true;
          services = {
            app = {
              command = "sleep 30";
              workingDir = ".";
              readiness = {
                type = "exec";
                command = "true";
                initialDelaySeconds = 1;
                periodSeconds = 1;
                timeoutSeconds = 1;
                failureThreshold = 1;
              };
            };
          };
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots supervisor;
        postgres = null;
        nginx = null;
        minio = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "supervisor-management-test";
        env = {
          "''${project.project.envVar}" = "dev";
          "''${project.project.slotVar}" = "0";
        };
        useDeps = false;
        script = import ./tests/framework/fixtures/supervisor/management.nix {
          supervisorStartDaemon = toString supervisor.startDaemon;
          supervisorStop = toString supervisor.stop;
          supervisorHealth = toString supervisor.health;
          supervisorRestart = toString supervisor.restart;
          supervisorRotateLogs = toString supervisor.rotateLogs;
          supervisorIsRunning = toString supervisor.isRunning;
          supervisorLogs = toString supervisor.logs;
        };
      }
    NIX
    )

    SUP_MGMT_SCRIPT=$(build_expr "$SUP_MGMT_EXPR")
    SUP_MGMT_LOG="$WORKDIR/supervisor-management.log"
    set +e
    (cd "$SUP_MGMT_DIR" && "$SUP_MGMT_SCRIPT" >"$SUP_MGMT_LOG" 2>&1)
    SUP_MGMT_RC=$?
    set -e
    if [ "$SUP_MGMT_RC" -ne 0 ]; then
      echo "Supervisor management fixture failed (rc=$SUP_MGMT_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$SUP_MGMT_LOG" 80
      exit "$SUP_MGMT_RC"
    fi

    log "run registry background/timeout"
    REG_BG_DIR="$WORKDIR/registry-background-timeout"
    mkdir -p "$REG_BG_DIR"
    REG_BG_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        ci.runsRoot = "/tmp/nixfied-framework-test-runs";
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots;
        postgres = null;
        nginx = null;
        minio = null;
        supervisor = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
    in
      lib.mkAppScript {
        name = "registry-bg-timeout-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/registry/background-timeout.nix {
          runRegistryStart = toString lib.runRegistryStart;
          runsRoot = "/tmp/nixfied-framework-test-runs";
        };
      }
    NIX
    )

    REG_BG_SCRIPT=$(build_expr "$REG_BG_EXPR")
    REG_BG_LOG="$WORKDIR/registry-background-timeout.log"
    set +e
    (cd "$REG_BG_DIR" && "$REG_BG_SCRIPT" >"$REG_BG_LOG" 2>&1)
    REG_BG_RC=$?
    set -e
    if [ "$REG_BG_RC" -ne 0 ]; then
      echo "Registry background/timeout fixture failed (rc=$REG_BG_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$REG_BG_LOG" 80
      exit "$REG_BG_RC"
    fi

    log "lib parallel"
    LIB_PAR_DIR="$WORKDIR/lib-parallel"
    mkdir -p "$LIB_PAR_DIR"
    LIB_PAR_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = base;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      parallel = import ./nixfied/.framework/lib/parallel.nix { inherit pkgs; };
      runnerOk = parallel.mkParallelRunner [
        "echo first"
        "echo second"
      ];
      runnerFail = parallel.mkParallelRunner [
        "echo passing-command"
        "echo failing-command >&2; exit 3"
      ];
    in
      lib.mkAppScript {
        name = "lib-parallel-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/lib/parallel.nix {
          parallelRunnerOk = toString runnerOk;
          parallelRunnerFail = toString runnerFail;
        };
      }
    NIX
    )

    LIB_PAR_SCRIPT=$(build_expr "$LIB_PAR_EXPR")
    LIB_PAR_LOG="$WORKDIR/lib-parallel.log"
    set +e
    (cd "$LIB_PAR_DIR" && "$LIB_PAR_SCRIPT" >"$LIB_PAR_LOG" 2>&1)
    LIB_PAR_RC=$?
    set -e
    if [ "$LIB_PAR_RC" -ne 0 ]; then
      echo "Lib parallel fixture failed (rc=$LIB_PAR_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$LIB_PAR_LOG" 80
      exit "$LIB_PAR_RC"
    fi

    log "lib port utils"
    LIB_PORT_DIR="$WORKDIR/lib-port-utils"
    mkdir -p "$LIB_PORT_DIR"
    LIB_PORT_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        tooling.runtimePackages =
          (base.tooling.runtimePackages or [ ])
          ++ [
            pkgs.lsof
            pkgs.netcat
          ];
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      portUtils = import ./nixfied/.framework/lib/port-utils.nix { inherit pkgs; };
    in
      lib.mkAppScript {
        name = "lib-port-utils-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/lib/port-utils.nix {
          portCleanup = toString portUtils.mkPortCleanup;
          portConflictChecker = toString portUtils.mkPortConflictChecker;
        };
      }
    NIX
    )

    LIB_PORT_SCRIPT=$(build_expr "$LIB_PORT_EXPR")
    LIB_PORT_LOG="$WORKDIR/lib-port-utils.log"
    set +e
    (cd "$LIB_PORT_DIR" && "$LIB_PORT_SCRIPT" >"$LIB_PORT_LOG" 2>&1)
    LIB_PORT_RC=$?
    set -e
    if [ "$LIB_PORT_RC" -ne 0 ]; then
      echo "Lib port-utils fixture failed (rc=$LIB_PORT_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$LIB_PORT_LOG" 80
      exit "$LIB_PORT_RC"
    fi

    log "lib process"
    LIB_PROC_DIR="$WORKDIR/lib-process"
    mkdir -p "$LIB_PROC_DIR"
    LIB_PROC_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = base;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      process = import ./nixfied/.framework/lib/process.nix { inherit pkgs; };
      processManager = process.mkProcessManager {
        processName = "fixture-process";
        startupScript = "sleep 30 &\nCHILD_PID=$!\nwait \"$CHILD_PID\"";
        cleanupHook = "echo \"cleaned\" >> \"$PROCESS_FIXTURE_MARKER\"";
      };
    in
      lib.mkAppScript {
        name = "lib-process-test";
        env = { };
        useDeps = false;
        script = import ./tests/framework/fixtures/lib/process.nix {
          processManager = toString processManager;
        };
      }
    NIX
    )

    LIB_PROC_SCRIPT=$(build_expr "$LIB_PROC_EXPR")
    LIB_PROC_LOG="$WORKDIR/lib-process.log"
    set +e
    (cd "$LIB_PROC_DIR" && "$LIB_PROC_SCRIPT" >"$LIB_PROC_LOG" 2>&1)
    LIB_PROC_RC=$?
    set -e
    if [ "$LIB_PROC_RC" -ne 0 ]; then
      echo "Lib process fixture failed (rc=$LIB_PROC_RC)." >&2
      echo "" >&2
      echo "Fixture output (last 80 lines):" >&2
      print_log_tail "$LIB_PROC_LOG" 80
      exit "$LIB_PROC_RC"
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
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      hooks = import ./nixfied/.framework/hooks.nix { inherit pkgs project slots; postgres = null; nginx = null; };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      ciEntry = import ./nixfied/.framework/ci.nix { inherit pkgs project lib; };
    in
      ciEntry.scriptDrv
    NIX
    )

    CI_SJ_SCRIPT=$(build_expr "$CI_SJ_EXPR")
    CI_SJ_DIR="$WORKDIR/ci-summary-json"
    CI_SJ_LOG="$WORKDIR/ci-summary-json.log"
    mkdir -p "$CI_SJ_DIR"
    set +e
    (cd "$CI_SJ_DIR" && CI_ARTIFACTS_DIR=".ci-artifacts" "$CI_SJ_SCRIPT" --mode check > "$CI_SJ_LOG" 2>&1)
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
        modules.minio.enable = true;
        modules.reth.enable = true;
        modules.helios.enable = true;
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres = import ./nixfied/.framework/postgres { inherit pkgs project slots; };
      nginx = import ./nixfied/.framework/nginx { inherit pkgs project slots; };
      minio = import ./nixfied/.framework/minio { inherit pkgs project slots; };
      reth = import ./nixfied/.framework/reth { inherit pkgs project slots; };
      helios = import ./nixfied/.framework/helios { inherit pkgs project slots; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
      serviceApis = {
        postgres = postgres.publicApi;
        nginx = nginx.publicApi;
        minio = minio.publicApi;
        reth = reth.publicApi;
        helios = helios.publicApi;
      };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres nginx minio reth helios supervisor serviceApis;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      moduleApps = import ./nixfied/.framework/internal/module-apps.nix {
        inherit pkgs project lib supervisor slots serviceApis;
      };
    in
      pkgs.writeText "module-app-names" (builtins.concatStringsSep "\n" (builtins.attrNames moduleApps))
    NIX
    )

    MODAPP_NAMES_FILE=$(build_expr "$MODAPP_EXPR")
    assert_contains "$MODAPP_NAMES_FILE" "service::postgres::start"
    assert_contains "$MODAPP_NAMES_FILE" "service::nginx::start"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::start"
    assert_contains "$MODAPP_NAMES_FILE" "service::reth::start"
    assert_contains "$MODAPP_NAMES_FILE" "service::helios::start"
    assert_contains "$MODAPP_NAMES_FILE" "service::postgres::health"
    assert_contains "$MODAPP_NAMES_FILE" "service::nginx::health"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::health"
    assert_contains "$MODAPP_NAMES_FILE" "service::reth::health"
    assert_contains "$MODAPP_NAMES_FILE" "service::helios::health"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::full-start"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::full-start-test"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::export-s3-env"
    assert_contains "$MODAPP_NAMES_FILE" "service::minio::bucket-ensure"
    assert_contains "$MODAPP_NAMES_FILE" "service::reth::full-start"
    assert_contains "$MODAPP_NAMES_FILE" "service::reth::full-start-test"
    assert_contains "$MODAPP_NAMES_FILE" "service::helios::full-start"
    assert_contains "$MODAPP_NAMES_FILE" "service::helios::full-start-test"
    assert_contains "$MODAPP_NAMES_FILE" "up"
    assert_contains "$MODAPP_NAMES_FILE" "svc-health"
    assert_contains "$MODAPP_NAMES_FILE" "check-ports"

    log "strict slot/env enforcement"
    STRICT_SLOT_ENV_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules.postgres.enable = true;
        modules.nginx.enable = true;
        modules.minio.enable = true;
        modules.reth.enable = true;
        modules.helios.enable = true;
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres = import ./nixfied/.framework/postgres { inherit pkgs project slots; };
      nginx = import ./nixfied/.framework/nginx { inherit pkgs project slots; };
      minio = import ./nixfied/.framework/minio { inherit pkgs project slots; };
      reth = import ./nixfied/.framework/reth { inherit pkgs project slots; };
      helios = import ./nixfied/.framework/helios { inherit pkgs project slots; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
      serviceApis = {
        postgres = postgres.publicApi;
        nginx = nginx.publicApi;
        minio = minio.publicApi;
        reth = reth.publicApi;
        helios = helios.publicApi;
      };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres nginx minio reth helios supervisor serviceApis;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      moduleApps = import ./nixfied/.framework/internal/module-apps.nix {
        inherit pkgs project lib supervisor slots serviceApis;
      };
    in
      pkgs.writeText "strict-slot-env-apps" (
        "UP=" + moduleApps.up.program + "\n"
        + "POSTGRES_LIST_INSTANCES=" + moduleApps."service::postgres::list-instances".program + "\n"
        + "POSTGRES_CHECK_PORT=" + moduleApps."service::postgres::check-port".program + "\n"
        + "HOOK_POSTGRES_LIST_INSTANCES=" + hooks.env.POSTGRES_LIST_INSTANCES + "\n"
        + "HOOK_POSTGRES_CHECK_PORT=" + hooks.env.POSTGRES_CHECK_PORT + "\n"
        + "REQUIRE_SLOT_ENV=" + hooks.env.REQUIRE_SLOT_ENV + "\n"
      )
    NIX
    )

    STRICT_SLOT_ENV_FILE=$(build_expr "$STRICT_SLOT_ENV_EXPR")
    STRICT_UP_SCRIPT=$(grep 'UP=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*UP=//')
    STRICT_PG_LIST_SCRIPT=$(grep 'POSTGRES_LIST_INSTANCES=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*POSTGRES_LIST_INSTANCES=//')
    STRICT_PG_CHECK_PORT_SCRIPT=$(grep 'POSTGRES_CHECK_PORT=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*POSTGRES_CHECK_PORT=//')
    STRICT_HOOK_PG_LIST_SCRIPT=$(grep 'HOOK_POSTGRES_LIST_INSTANCES=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*HOOK_POSTGRES_LIST_INSTANCES=//')
    STRICT_HOOK_PG_CHECK_PORT_SCRIPT=$(grep 'HOOK_POSTGRES_CHECK_PORT=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*HOOK_POSTGRES_CHECK_PORT=//')
    STRICT_REQUIRE_SLOT_ENV_SCRIPT=$(grep 'REQUIRE_SLOT_ENV=' "$STRICT_SLOT_ENV_FILE" | head -1 | sed 's/^[[:space:]]*REQUIRE_SLOT_ENV=//')

    STRICT_UP_MISSING_ENV_LOG="$WORKDIR/strict-up-missing-env.log"
    set +e
    NIX_ENV=0 "$STRICT_UP_SCRIPT" > "$STRICT_UP_MISSING_ENV_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected up app to fail when PROJECT_ENV is missing"
    fi
    assert_contains "$STRICT_UP_MISSING_ENV_LOG" "PROJECT_ENV must be set"

    STRICT_REQUIRE_MISSING_SLOT_LOG="$WORKDIR/strict-require-missing-slot.log"
    set +e
    PROJECT_ENV=dev "$STRICT_REQUIRE_SLOT_ENV_SCRIPT" > "$STRICT_REQUIRE_MISSING_SLOT_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -ne 0 ]; then
      fail "expected REQUIRE_SLOT_ENV to default NIX_ENV when missing"
    fi
    assert_contains "$STRICT_REQUIRE_MISSING_SLOT_LOG" "INFO: default slot selected"
    assert_contains "$STRICT_REQUIRE_MISSING_SLOT_LOG" "SLOT=0"

    STRICT_PG_MISSING_ENV_LOG="$WORKDIR/strict-pg-list-missing-env.log"
    set +e
    NIX_ENV=0 "$STRICT_PG_LIST_SCRIPT" > "$STRICT_PG_MISSING_ENV_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected service::postgres::list-instances to fail when PROJECT_ENV is missing"
    fi
    assert_contains "$STRICT_PG_MISSING_ENV_LOG" "PROJECT_ENV must be set"

    PROJECT_ENV=dev NIX_ENV=0 "$STRICT_PG_LIST_SCRIPT" >/dev/null

    STRICT_PG_HOOK_MISSING_ENV_LOG="$WORKDIR/strict-pg-hook-list-missing-env.log"
    set +e
    REQUIRE_SLOT_ENV="$STRICT_REQUIRE_SLOT_ENV_SCRIPT" NIX_ENV=0 "$STRICT_HOOK_PG_LIST_SCRIPT" > "$STRICT_PG_HOOK_MISSING_ENV_LOG" 2>&1
    RC=$?
    set -e
    if [ "$RC" -eq 0 ]; then
      fail "expected POSTGRES_LIST_INSTANCES hook to fail when PROJECT_ENV is missing"
    fi
    assert_contains "$STRICT_PG_HOOK_MISSING_ENV_LOG" "PROJECT_ENV must be set"

    REQUIRE_SLOT_ENV="$STRICT_REQUIRE_SLOT_ENV_SCRIPT" PROJECT_ENV=dev NIX_ENV=0 "$STRICT_HOOK_PG_LIST_SCRIPT" >/dev/null

    log "service arg forwarding parity"
    STRICT_PORT_ARG=65529

    STRICT_PG_CHECK_PORT_LOG="$WORKDIR/strict-pg-check-port-app.log"
    set +e
    PROJECT_ENV=dev NIX_ENV=0 "$STRICT_PG_CHECK_PORT_SCRIPT" "$STRICT_PORT_ARG" > "$STRICT_PG_CHECK_PORT_LOG" 2>&1
    RC=$?
    set -e
    if grep -q "usage: postgres-check-port <port>" "$STRICT_PG_CHECK_PORT_LOG"; then
      fail "expected service::postgres::check-port to receive forwarded args"
    fi
    assert_contains "$STRICT_PG_CHECK_PORT_LOG" "$STRICT_PORT_ARG"
    if [ "$RC" -ne 0 ] && ! grep -q "Port $STRICT_PORT_ARG is in use by PID(s):" "$STRICT_PG_CHECK_PORT_LOG"; then
      fail "unexpected failure from service::postgres::check-port with forwarded args"
    fi

    STRICT_PG_HOOK_CHECK_PORT_LOG="$WORKDIR/strict-pg-check-port-hook.log"
    set +e
    REQUIRE_SLOT_ENV="$STRICT_REQUIRE_SLOT_ENV_SCRIPT" PROJECT_ENV=dev NIX_ENV=0 "$STRICT_HOOK_PG_CHECK_PORT_SCRIPT" "$STRICT_PORT_ARG" > "$STRICT_PG_HOOK_CHECK_PORT_LOG" 2>&1
    RC=$?
    set -e
    if grep -q "usage: postgres-check-port <port>" "$STRICT_PG_HOOK_CHECK_PORT_LOG"; then
      fail "expected POSTGRES_CHECK_PORT hook to receive forwarded args"
    fi
    assert_contains "$STRICT_PG_HOOK_CHECK_PORT_LOG" "$STRICT_PORT_ARG"
    if [ "$RC" -ne 0 ] && ! grep -q "Port $STRICT_PORT_ARG is in use by PID(s):" "$STRICT_PG_HOOK_CHECK_PORT_LOG"; then
      fail "unexpected failure from POSTGRES_CHECK_PORT hook with forwarded args"
    fi

    MODAPP_DISABLED_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = base;
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots supervisor;
        postgres = null;
        nginx = null;
        minio = null;
      };
      lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
      moduleApps = import ./nixfied/.framework/internal/module-apps.nix {
        inherit pkgs project lib slots supervisor;
        serviceApis = { };
      };
    in
      pkgs.writeText "module-app-names-disabled" (builtins.concatStringsSep "\n" (builtins.attrNames moduleApps))
    NIX
    )

    MODAPP_DISABLED_FILE=$(build_expr "$MODAPP_DISABLED_EXPR")
    if grep -q "service::postgres::start" "$MODAPP_DISABLED_FILE"; then
      fail "service::postgres::start should not be present when postgres is disabled"
    fi
    if grep -q "service::nginx::start" "$MODAPP_DISABLED_FILE"; then
      fail "service::nginx::start should not be present when nginx is disabled"
    fi
    if grep -q "service::minio::start" "$MODAPP_DISABLED_FILE"; then
      fail "service::minio::start should not be present when minio is disabled"
    fi
    if grep -q "service::reth::start" "$MODAPP_DISABLED_FILE"; then
      fail "service::reth::start should not be present when reth is disabled"
    fi
    if grep -q "service::helios::start" "$MODAPP_DISABLED_FILE"; then
      fail "service::helios::start should not be present when helios is disabled"
    fi
    assert_contains "$MODAPP_DISABLED_FILE" "check-ports"
    assert_contains "$MODAPP_DISABLED_FILE" "ports"

    log "service hooks"
    SERVICE_HOOKS_EXPR=$(cat <<'NIX'
    { root, system }:
    let
      flake = builtins.getFlake root;
      pkgs = flake.inputs.nixpkgs.legacyPackages.''${system};
      base = import ./nixfied/project { inherit pkgs; };
      project = pkgs.lib.recursiveUpdate base {
        modules = {
          postgres.enable = true;
          nginx.enable = true;
          minio.enable = true;
          reth.enable = true;
          helios.enable = true;
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      postgres = import ./nixfied/.framework/postgres { inherit pkgs project slots; };
      nginx = import ./nixfied/.framework/nginx { inherit pkgs project slots; };
      minio = import ./nixfied/.framework/minio { inherit pkgs project slots; };
      reth = import ./nixfied/.framework/reth { inherit pkgs project slots; };
      helios = import ./nixfied/.framework/helios { inherit pkgs project slots; };
      serviceApis = {
        postgres = postgres.publicApi;
        nginx = nginx.publicApi;
        minio = minio.publicApi;
        reth = reth.publicApi;
        helios = helios.publicApi;
      };
      hooks = import ./nixfied/.framework/hooks.nix {
        inherit pkgs project slots postgres nginx minio reth helios serviceApis;
        supervisor = null;
      };
      names = builtins.attrNames hooks.env;
      selected =
        builtins.filter (
          n:
          builtins.substring 0 9 n == "POSTGRES_"
          || builtins.substring 0 6 n == "NGINX_"
          || builtins.substring 0 6 n == "MINIO_"
          || builtins.substring 0 5 n == "RETH_"
          || builtins.substring 0 7 n == "HELIOS_"
        ) names;
    in
      pkgs.writeText "service-hooks" (builtins.concatStringsSep "\n" (
        builtins.map (
          name:
          let
            value = builtins.getAttr name hooks.env;
          in
          "''${name}= ''${value}"
        ) selected
      ))
    NIX
    )

    SERVICE_HOOKS_FILE=$(build_expr "$SERVICE_HOOKS_EXPR")
    assert_contains "$SERVICE_HOOKS_FILE" "POSTGRES_START="
    assert_contains "$SERVICE_HOOKS_FILE" "POSTGRES_HEALTH="
    assert_contains "$SERVICE_HOOKS_FILE" "POSTGRES_READY="
    assert_contains "$SERVICE_HOOKS_FILE" "NGINX_START="
    assert_contains "$SERVICE_HOOKS_FILE" "NGINX_HEALTH="
    assert_contains "$SERVICE_HOOKS_FILE" "NGINX_READY="
    assert_contains "$SERVICE_HOOKS_FILE" "MINIO_START="
    assert_contains "$SERVICE_HOOKS_FILE" "MINIO_HEALTH="
    assert_contains "$SERVICE_HOOKS_FILE" "MINIO_READY="
    assert_contains "$SERVICE_HOOKS_FILE" "MINIO_BUCKET_LIST="
    assert_contains "$SERVICE_HOOKS_FILE" "RETH_START="
    assert_contains "$SERVICE_HOOKS_FILE" "RETH_HEALTH="
    assert_contains "$SERVICE_HOOKS_FILE" "RETH_READY="
    assert_contains "$SERVICE_HOOKS_FILE" "HELIOS_START="
    assert_contains "$SERVICE_HOOKS_FILE" "HELIOS_HEALTH="
    assert_contains "$SERVICE_HOOKS_FILE" "HELIOS_READY="
    assert_contains "$SERVICE_HOOKS_FILE" "/nix/store/"

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
              readiness = {
                type = "exec";
                command = "echo ready";
                initialDelaySeconds = 1;
                periodSeconds = 1;
                timeoutSeconds = 1;
                failureThreshold = 1;
              };
            };
          };
        };
      };
      slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };
      supervisor = import ./nixfied/.framework/supervisor { inherit pkgs project slots; };
      hooks = import ./nixfied/.framework/hooks.nix {
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
    assert_contains "$SUP_HOOKS_FILE" "SUPERVISOR_HEALTH="
    # Verify they point to nix store paths
    assert_contains "$SUP_HOOKS_FILE" "/nix/store/"

    if [ "$PROFILE" = "full" ] || [ "''${FRAMEWORK_ISOLATION:-}" = "1" ]; then
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
      details = "Runs the Nixfied framework integration test suite (intended for framework development). Supports --profile and --summary-json options.";
      usage = [
        "nix run .#framework::test"
        "nix run .#framework::test -- --profile full"
        "nix run .#framework::test -- --summary-json /tmp/framework-test-summary.json"
      ];
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
