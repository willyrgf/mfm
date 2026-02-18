# PostgreSQL lifecycle management - init, start, stop, setupDb, fullStart
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  cfg = project.modules.postgres or { };
  slotEnvRuntime = import ../lib/slot-env-runtime.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  postgres = cfg.package or pkgs.postgresql_16;
  portKey = cfg.portKey or "postgres";
  portVar = slots.portVarName portKey;
  dataDirName = cfg.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;
  database = cfg.database or "app";
  testDatabase = cfg.testDatabase or "${database}_test";
  extensions = config.extensions or [ ];
  pgRuntimePrelude = defaultDb: ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfoJson;
      exportVars = false;
    }}
    ${slotEnvRuntime.readJsonField {
      targetVar = "RUN_DIR";
      jsonVar = "SLOT_INFO_JSON_OUT";
      jqExpr = ".directories.run";
    }}

    PORT_VAR="${portVar}"
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "_PGPORT_RESOLVED";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$PORT_VAR";
    }}
    export PGPORT="''${PGPORT:-$_PGPORT_RESOLVED}"
    export PGDATA="''${PGDATA:-${pgdataExpr}}"
    # Keep the unix socket path short. In CI (and on some systems with long TMPDIR paths),
    # putting sockets under $PGDATA can exceed the 107-byte sockaddr_un.sun_path limit and
    # prevent PostgreSQL from starting.
    SOCKET_HASH=$(printf '%s' "''${RUN_DIR:-$PGDATA}" | ${pkgs.coreutils}/bin/cksum | ${pkgs.coreutils}/bin/cut -d ' ' -f1)
    export PGSOCKET_DIR="''${PGSOCKET_DIR:-/tmp/nixfied-pg-$SOCKET_HASH}"
    export PGDATABASE="''${PGDATABASE:-${defaultDb}}"

    if [ -z "''${PGPORT:-}" ] || [ -z "''${PGDATA:-}" ]; then
      log_error "Failed to resolve PostgreSQL runtime variables (PGPORT/PGDATA)"
      exit 1
    fi

    case "$PGPORT" in
      *[!0-9]*)
        log_error "PGPORT must be numeric (got '$PGPORT')"
        exit 1
        ;;
    esac

    ${observability.mkEmitServiceEventFunction "postgres"}
  '';

  ensureConfigPort = ''
    ensure_config_port() {
      local conf="$1"

      if [ ! -f "$conf" ]; then
        log_error "missing postgresql.conf path=$conf"
        exit 1
      fi

      if ${pkgs.gnugrep}/bin/grep -Eq '^[[:space:]]*port[[:space:]]*=' "$conf"; then
        ${pkgs.gnused}/bin/sed -i -E "s|^[[:space:]]*port[[:space:]]*=.*$|port = $PGPORT|g" "$conf"
      else
        printf '\nport = %s\n' "$PGPORT" >> "$conf"
      fi
    }
  '';

  init = pkgs.writeShellScript "postgres-init" ''
    ${loggingPrelude}

    set -euo pipefail

    ${pgRuntimePrelude database}
    ${ensureConfigPort}

    mkdir -p "$PGDATA"

    if [ -f "$PGDATA/PG_VERSION" ]; then
      log_ok "PostgreSQL already initialized at $PGDATA"
      exit 0
    fi

    log_info "Initializing PostgreSQL at $PGDATA"
    ${postgres}/bin/initdb -D "$PGDATA" -U postgres --no-locale --encoding=UTF8 -A trust

    # Determine environment-specific config
    CONF_ENV="''${ENV:-dev}"
    case "$CONF_ENV" in
      prod)
        cat > "$PGDATA/postgresql.conf" <<'PGCONF'
    ${config.prodConf}
    PGCONF
        ;;
      test)
        cat > "$PGDATA/postgresql.conf" <<'PGCONF'
    ${config.testConf}
    PGCONF
        ;;
      *)
        cat > "$PGDATA/postgresql.conf" <<'PGCONF'
    ${config.devConf}
    PGCONF
        ;;
    esac

    ensure_config_port "$PGDATA/postgresql.conf"
    if ! ${postgres}/bin/postgres -D "$PGDATA" -C port >/dev/null 2>&1; then
      log_error "PostgreSQL configuration invalid after init pgdata=$PGDATA"
      exit 1
    fi

    cat > "$PGDATA/pg_hba.conf" <<'EOF'
    # TYPE  DATABASE        USER  ADDRESS       METHOD
    local   all             all                 trust
    host    all             all   127.0.0.1/32  trust
    host    all             all   ::1/128       trust
    EOF
  '';

  start = pkgs.writeShellScript "postgres-start" ''
    ${loggingPrelude}

    set -euo pipefail

    ${pgRuntimePrelude database}
    ${ensureConfigPort}

    if [ ! -f "$PGDATA/postgresql.conf" ]; then
      log_error "PostgreSQL not initialized at $PGDATA (missing postgresql.conf)"
      echo "   Run postgres init first: nix run .#svc::postgres::init" >&2
      exit 1
    fi
    ensure_config_port "$PGDATA/postgresql.conf"

    if ${postgres}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      # Verify the running instance is ours by checking PGDATA
      if [ -f "$PGDATA/postmaster.pid" ]; then
        RUN_PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
        emit_service_event service_ready ready --pid "$RUN_PID" --log-path "$PGDATA/postgres.log"
        log_ok "PostgreSQL already running on port $PGPORT"
        exit 0
      else
        log_warn "Port $PGPORT in use by a different PostgreSQL instance"
        if [ "''${CI:-}" = "true" ] || [ "''${AUTO_STOP_CONFLICTING:-}" = "1" ]; then
          echo "   Auto-stopping conflicting instance (CI mode)..." >&2
          lsof -ti:$PGPORT 2>/dev/null | xargs kill -TERM 2>/dev/null || true
          sleep 2
        else
          echo "   Use 'run_hook SVC_POSTGRES_CHECK_PORT' to investigate" >&2
          exit 1
        fi
      fi
    fi

    # Clean up stale PID file
    if [ -f "$PGDATA/postmaster.pid" ]; then
      STALE_PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
      if [ -n "$STALE_PID" ] && ! kill -0 "$STALE_PID" 2>/dev/null; then
        log_info "Removing stale PID file (PID $STALE_PID not running)"
        rm -f "$PGDATA/postmaster.pid"
      fi
    fi

    if [ -z "''${PGSOCKET_DIR:-}" ]; then
      PGSOCKET_DIR="/tmp"
    fi
    mkdir -p "$PGSOCKET_DIR"
    chmod 700 "$PGSOCKET_DIR" 2>/dev/null || true

    log_info "Starting PostgreSQL on port $PGPORT"
    emit_service_event service_starting starting --log-path "$PGDATA/postgres.log"
    ${postgres}/bin/pg_ctl -D "$PGDATA" -l "$PGDATA/postgres.log" -o "-p $PGPORT -k $PGSOCKET_DIR" start

    for i in $(seq 1 60); do
      if ${postgres}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
        RUN_PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
        emit_service_event service_ready ready --pid "$RUN_PID" --log-path "$PGDATA/postgres.log"
        log_ok "PostgreSQL ready on port $PGPORT"
        exit 0
      fi
      sleep 0.5
    done

    emit_service_event service_degraded degraded \
      --log-path "$PGDATA/postgres.log" \
      --wait-reason "failed_readiness" \
      --last-error "postgres did not become ready in startup window"
    log_error "PostgreSQL failed to start. Check $PGDATA/postgres.log"
    if [ -f "$PGDATA/postgres.log" ]; then
      log_info "postgres log tail path=$PGDATA/postgres.log lines=20"
      tail -20 "$PGDATA/postgres.log" >&2 || true
    else
      log_warn "postgres log file missing path=$PGDATA/postgres.log"
    fi
    exit 1
  '';

  stop = pkgs.writeShellScript "postgres-stop" ''
    ${loggingPrelude}

    set -euo pipefail
    ${pgRuntimePrelude database}

    if [ -n "''${PGDATA:-}" ] && [ -f "$PGDATA/postmaster.pid" ]; then
      RUN_PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
      log_stop "PostgreSQL at $PGDATA"
      ${postgres}/bin/pg_ctl -D "$PGDATA" stop -m fast 2>/dev/null || true
      emit_service_event service_stopped stopped --pid "$RUN_PID" --log-path "$PGDATA/postgres.log"
    else
      emit_service_event service_stopped stopped
    fi
  '';

  setupDb = pkgs.writeShellScript "postgres-setup-db" ''
    ${loggingPrelude}

    set -euo pipefail

    ${pgRuntimePrelude database}

    log_info "Setting up database '$PGDATABASE'"

    ${postgres}/bin/psql -h localhost -p "$PGPORT" -U postgres -d postgres -c \
      "DO \$\$ BEGIN CREATE ROLE postgres WITH LOGIN SUPERUSER PASSWORD 'postgres'; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;" 2>/dev/null || true

    ${postgres}/bin/createdb -h localhost -p "$PGPORT" -U postgres "$PGDATABASE" 2>/dev/null || true

    if [ -n "${pkgs.lib.concatStringsSep " " extensions}" ]; then
      for ext in ${pkgs.lib.concatStringsSep " " extensions}; do
        ${postgres}/bin/psql -h localhost -p "$PGPORT" -U postgres -d "$PGDATABASE" \
          -c "CREATE EXTENSION IF NOT EXISTS $ext;" 2>/dev/null || true
      done
    fi

    log_ok "Database '$PGDATABASE' ready"
  '';

  fullStart = pkgs.writeShellScript "postgres-full-start" ''
    ${loggingPrelude}

    set -euo pipefail
    ${pgRuntimePrelude database}

    log_info "Slot $SLOT, env $ENV (PGPORT=$PGPORT)"

    ${init}
    ${start}
    ${setupDb}

    echo "PGPORT=$PGPORT"
    echo "PGDATA=$PGDATA"
    echo "PGDATABASE=$PGDATABASE"
  '';

  fullStartTest = pkgs.writeShellScript "postgres-full-start-test" ''
    ${loggingPrelude}

    set -euo pipefail
    ${pgRuntimePrelude testDatabase}
    export PGDATABASE="''${PGDATABASE:-${testDatabase}}"

    ${init}
    ${start}
    ${setupDb}
  '';

  listInstances = pkgs.writeShellScript "postgres-list-instances" ''
    ${loggingPrelude}

    set -euo pipefail
    echo "PostgreSQL instances:"
    echo ""
    for pidfile in $(find "''${XDG_DATA_HOME:-$HOME/.local/share}" -name "postmaster.pid" 2>/dev/null || true); do
      PGDATA_DIR=$(dirname "$pidfile")
      PID=$(head -1 "$pidfile" 2>/dev/null || echo "unknown")
      PORT=$(sed -n '4p' "$pidfile" 2>/dev/null || echo "unknown")
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        STATUS="running"
      else
        STATUS="stale"
      fi
      echo "  $PGDATA_DIR (PID: $PID, Port: $PORT, Status: $STATUS)"
    done
  '';

in
{
  inherit
    postgres
    init
    start
    stop
    setupDb
    fullStart
    fullStartTest
    listInstances
    ;
}
