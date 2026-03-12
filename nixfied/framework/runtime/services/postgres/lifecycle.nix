# PostgreSQL lifecycle management - init, start, stop, setupDb, fullStart
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  managedServiceLifecycle = import ../../helpers/managed-service-lifecycle.nix { inherit pkgs; };
  probeCommands = import ../../helpers/probe-commands.nix { inherit pkgs; };
  slotEnvRuntime = import ../../helpers/slot-env-runtime.nix { inherit pkgs; };
  runtimeEvents = import ../../helpers/runtime-events.nix { inherit pkgs project; };
  observability = import ../../helpers/service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };
  postgres = config.package or pkgs.postgresql_16;
  portKey = config.portKey or "postgres";
  portVar = slots.portVarName portKey;
  dataDirName = config.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;
  database = config.database or "app";
  testDatabase = config.testDatabase or "${database}_test";
  extensions = config.extensions or [ ];
  mkWrappedScript =
    {
      name,
      runtimePrelude ? "",
      body,
    }:
    managedServiceLifecycle.mkWrappedScript {
      inherit
        name
        loggingPrelude
        runtimePrelude
        body
        ;
    };
  mkPgScript =
    {
      name,
      defaultDb ? database,
      body,
    }:
    mkWrappedScript {
      inherit name body;
      runtimePrelude = pgRuntimePrelude defaultDb;
    };
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

  selectConfigTemplate = ''
    select_config_template() {
      case "''${1:-dev}" in
        prod)
          printf '%s\n' '${config.prodConfFile}'
          ;;
        test)
          printf '%s\n' '${config.testConfFile}'
          ;;
        *)
          printf '%s\n' '${config.devConfFile}'
          ;;
      esac
    }
  '';

  init = mkPgScript {
    name = "postgres-init";
    body = ''
      ${ensureConfigPort}
      ${selectConfigTemplate}

      mkdir -p "$PGDATA"

      if [ -f "$PGDATA/PG_VERSION" ]; then
        log_ok "PostgreSQL already initialized at $PGDATA"
        exit 0
      fi

      log_info "Initializing PostgreSQL at $PGDATA"
      ${postgres}/bin/initdb -D "$PGDATA" -U postgres --no-locale --encoding=UTF8 -A trust

      # Determine environment-specific config
      CONF_ENV="''${ENV:-dev}"
      PGCONF_TEMPLATE="$(select_config_template "$CONF_ENV")"
      ${pkgs.coreutils}/bin/install -m 600 "$PGCONF_TEMPLATE" "$PGDATA/postgresql.conf"

      ensure_config_port "$PGDATA/postgresql.conf"
      if ! ${postgres}/bin/postgres -D "$PGDATA" -C port >/dev/null 2>&1; then
        log_error "PostgreSQL configuration invalid after init pgdata=$PGDATA"
        exit 1
      fi

      ${pkgs.coreutils}/bin/install -m 600 ${config.pgHbaConfFile} "$PGDATA/pg_hba.conf"
    '';
  };

  start = mkPgScript {
    name = "postgres-start";
    body = ''
      ${ensureConfigPort}

      if [ ! -f "$PGDATA/postgresql.conf" ]; then
        log_error "PostgreSQL not initialized at $PGDATA (missing postgresql.conf)"
        echo "   Run postgres init first: nix run .#svc::postgres::init" >&2
        exit 1
      fi
      ensure_config_port "$PGDATA/postgresql.conf"

      if ${
        probeCommands.pgIsReadyCmd {
          inherit postgres;
          portExpr = "$PGPORT";
        }
      } then
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
        if ${
          probeCommands.pgIsReadyCmd {
            inherit postgres;
            portExpr = "$PGPORT";
          }
        } then
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
      print_log_tail "$PGDATA/postgres.log" 20 "postgres"
      exit 1
    '';
  };

  stop = mkPgScript {
    name = "postgres-stop";
    body = ''
      if [ -n "''${PGDATA:-}" ] && [ -f "$PGDATA/postmaster.pid" ]; then
        RUN_PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
        log_stop "PostgreSQL at $PGDATA"
        ${postgres}/bin/pg_ctl -D "$PGDATA" stop -m fast 2>/dev/null || true
        emit_service_event service_stopped stopped --pid "$RUN_PID" --log-path "$PGDATA/postgres.log"
      else
        emit_service_event service_stopped stopped
      fi
    '';
  };

  restart = mkPgScript {
    name = "postgres-restart";
    body = ''
      ${stop}
      exec ${start}
    '';
  };

  status = managedServiceLifecycle.mkObservedStatusScript {
    name = "postgres-status";
    inherit loggingPrelude;
    runtimePrelude = pgRuntimePrelude database;
    runningStateBody = ''
      if ${
        probeCommands.pgIsReadyCmd {
          inherit postgres;
          portExpr = "$PGPORT";
        }
      } then
        RUNNING=true
      fi

      if [ -f "$PGDATA/postmaster.pid" ]; then
        PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
      fi
    '';
    statusMergeBlock = observability.mkStatusMergeBlock {
      service = "postgres";
      defaultLogPathExpr = ''"$PGDATA/postgres.log"'';
    };
    statusBody = observability.mkStatusLine {
      service = "postgres";
      beforeRunningFields = [
        "port=$PGPORT"
        "pgdata=$PGDATA"
      ];
    };
  };

  health = mkPgScript {
    name = "postgres-health";
    body = managedServiceLifecycle.mkSimpleProbeBody {
      probeCommand = probeCommands.pgIsReadyCmd {
        inherit postgres;
        portExpr = "$PGPORT";
      };
      successMessage = "PostgreSQL healthy port=$PGPORT";
      failureMessage = "PostgreSQL unhealthy port=$PGPORT";
    };
  };

  ready = mkPgScript {
    name = "postgres-ready";
    body = ''
      if ! ${
        probeCommands.pgIsReadyCmd {
          inherit postgres;
          portExpr = "$PGPORT";
        }
      } then
        log_error "PostgreSQL not ready port=$PGPORT (pg_isready failed)"
        exit 1
      fi

      if ${
        probeCommands.psqlQueryCmd {
          inherit postgres;
          portExpr = "$PGPORT";
          databaseExpr = "postgres";
          query = "select 1;";
        }
      } >/dev/null 2>&1; then
        log_ok "PostgreSQL ready port=$PGPORT"
        exit 0
      fi

      log_error "PostgreSQL not ready port=$PGPORT (query failed)"
      exit 1
    '';
  };

  readyTest = mkPgScript {
    name = "postgres-ready-test";
    defaultDb = testDatabase;
    body = ''
      export PGDATABASE="''${PGDATABASE:-${testDatabase}}"

      if ! ${
        probeCommands.pgIsReadyCmd {
          inherit postgres;
          portExpr = "$PGPORT";
        }
      } then
        log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (pg_isready failed)"
        exit 1
      fi

      if ! ${
        probeCommands.psqlQueryCmd {
          inherit postgres;
          portExpr = "$PGPORT";
          databaseExpr = "postgres";
          query = "select 1;";
        }
      } >/dev/null 2>&1; then
        log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (maintenance query failed)"
        exit 1
      fi

      if ${
        probeCommands.psqlQueryCmd {
          inherit postgres;
          portExpr = "$PGPORT";
          databaseExpr = "$PGDATABASE";
          query = "select 1;";
        }
      } >/dev/null 2>&1; then
        log_ok "PostgreSQL ready for test db port=$PGPORT database=$PGDATABASE"
        exit 0
      fi

      log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (database query failed)"
      exit 1
    '';
  };

  checkConfig = mkPgScript {
    name = "postgres-check-config";
    body = ''
      if [ ! -f "$PGDATA/postgresql.conf" ]; then
        log_error "missing postgresql.conf at $PGDATA"
        exit 1
      fi

      if ${postgres}/bin/postgres -D "$PGDATA" -C port >/dev/null 2>&1; then
        log_ok "PostgreSQL configuration valid pgdata=$PGDATA"
        exit 0
      fi

      log_error "PostgreSQL configuration invalid pgdata=$PGDATA"
      exit 1
    '';
  };

  setupDb = mkPgScript {
    name = "postgres-setup-db";
    body = ''
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
  };

  fullStart = mkPgScript {
    name = "postgres-full-start";
    body = ''
      log_info "Slot $SLOT, env $ENV (PGPORT=$PGPORT)"

      ${init}
      ${start}
      ${setupDb}

      echo "PGPORT=$PGPORT"
      echo "PGDATA=$PGDATA"
      echo "PGDATABASE=$PGDATABASE"
    '';
  };

  fullStartTest = mkPgScript {
    name = "postgres-full-start-test";
    defaultDb = testDatabase;
    body = ''
      export PGDATABASE="''${PGDATABASE:-${testDatabase}}"

      ${init}
      ${start}
      ${setupDb}
    '';
  };

  listInstances = mkWrappedScript {
    name = "postgres-list-instances";
    body = ''
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
  };

in
{
  inherit
    postgres
    init
    start
    stop
    restart
    status
    health
    ready
    readyTest
    checkConfig
    setupDb
    fullStart
    fullStartTest
    listInstances
    ;
}
