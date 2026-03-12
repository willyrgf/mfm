# Nginx lifecycle management - init, start, stop, reload
{
  pkgs,
  project,
  slots,
  config,
  templates,
  loggingPrelude,
}:

let
  serviceScripts = import ../../helpers/managed-service-lifecycle.nix { inherit pkgs; };
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
  nginx = templates.nginx;
  portVarHttp = slots.portVarName (config.portKeyHttp or "http");
  portVarHttps = slots.portVarName (config.portKeyHttps or "https");
  dataDirName = config.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;
  emitHelper = observability.mkEmitServiceEventFunction "nginx";
  runtimePrelude = ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfoJson;
      exportVars = false;
    }}

    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"

    ${slotEnvRuntime.readPortFromJson {
      targetVar = "HTTP_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$HTTP_PORT_VAR";
    }}
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "HTTPS_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$HTTPS_PORT_VAR";
    }}
    NGINX_DIR="${nginxDirExpr}"
    NGINX_PID_FILE="$NGINX_DIR/run/nginx.pid"
    NGINX_LOG_FILE="$NGINX_DIR/logs/error.log"
    SERVICE_DIR="$NGINX_DIR"
    SERVICE_PID_FILE="$NGINX_PID_FILE"
    SERVICE_LOG_FILE="$NGINX_LOG_FILE"

    if [ -z "$HTTP_PORT" ] || [ -z "$HTTPS_PORT" ]; then
      log_error "nginx port variables are not set (http/https)"
      exit 1
    fi

    ${emitHelper}
  '';

  generateSelfSignedCert = serviceScripts.mkWrappedScript {
    name = "nginx-generate-self-signed";
    inherit loggingPrelude;
    runtimePrelude = "";
    body = ''
      DOMAIN="$1"
      SSL_DIR="$2"

      CERT_DIR="$SSL_DIR/live/$DOMAIN"
      mkdir -p "$CERT_DIR"

      if [ -f "$CERT_DIR/fullchain.pem" ] && [ -f "$CERT_DIR/privkey.pem" ]; then
        exit 0
      fi

      ${pkgs.openssl}/bin/openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
        -keyout "$CERT_DIR/privkey.pem" \
        -out "$CERT_DIR/fullchain.pem" \
        -subj "/CN=$DOMAIN" \
        2>/dev/null
      cp "$CERT_DIR/fullchain.pem" "$CERT_DIR/chain.pem"
    '';
  };

  managedLifecycle = serviceScripts.mkPidFileManagedLifecycle {
    service = "nginx";
    inherit
      loggingPrelude
      runtimePrelude
      ;
    initBody = ''
      mkdir -p "$SERVICE_DIR/conf/sites-available"
      mkdir -p "$SERVICE_DIR/conf/sites-enabled"
      mkdir -p "$SERVICE_DIR/ssl/live/localhost"
      mkdir -p "$SERVICE_DIR/logs"
      mkdir -p "$SERVICE_DIR/html"
      mkdir -p "$SERVICE_DIR/run"

      ${pkgs.gnused}/bin/sed \
        -e "s|NGINX_DIR|$NGINX_DIR|g" \
        -e "s|HTTP_PORT|$HTTP_PORT|g" \
        -e "s|HTTPS_PORT|$HTTPS_PORT|g" \
        "${templates.nginxConfTemplate}" > "$NGINX_DIR/conf/nginx.conf"

      ${generateSelfSignedCert} "localhost" "$NGINX_DIR/ssl"
      log_ok "nginx initialized dir=$NGINX_DIR slot=$SLOT env=$ENV"
    '';
    checkConfigBody = ''
      CONF="$NGINX_DIR/conf/nginx.conf"

      if [ ! -f "$CONF" ]; then
        log_error "missing nginx config at $CONF"
        exit 1
      fi

      ${nginx}/bin/nginx -c "$CONF" -t 2>&1
      log_ok "nginx configuration valid conf=$CONF"
    '';
    startPreflight = ''
      CONF="$NGINX_DIR/conf/nginx.conf"

      if [ ! -f "$CONF" ]; then
        log_error "Nginx not initialized. Run nginx-init first."
        exit 1
      fi
    '';
    startCommand = ''
      ${nginx}/bin/nginx -c "$CONF" -g 'daemon off;' > "$LOG_FILE" 2>&1 &
    '';
    startAlreadyRunningBody = serviceScripts.mkReadyOutcomeBody {
      level = "ok";
      pidExpr = ''"$PID"'';
      message = "nginx already running pid=$PID http_port=$HTTP_PORT";
    };
    startPostLaunchBody = serviceScripts.mkStartupReadinessBody {
      probeCommand = probeCommands.tcpOpenCmd { portExpr = "$HTTP_PORT"; };
      serviceLabel = "nginx";
      degradedWaitReason = "failed_readiness";
      degradedLastError = "nginx failed health check during startup";
      failureMessage = "nginx failed to become healthy";
      successMessage = "nginx started pid=$CHILD_PID http_port=$HTTP_PORT https_port=$HTTPS_PORT";
    };
    startExitFailureBody = serviceScripts.mkProcessExitFailureBody {
      waitReason = "nginx_process_exit code=$RC";
      lastError = "nginx process exited non-zero";
    };
    stopRequestBody = ''
      if [ -f "$NGINX_DIR/conf/nginx.conf" ]; then
        ${nginx}/bin/nginx -c "$NGINX_DIR/conf/nginx.conf" -s quit 2>/dev/null || kill "$PID" 2>/dev/null || true
      else
        kill "$PID" 2>/dev/null || true
      fi
    '';
    statusMergeBlock = observability.mkStatusMergeBlock {
      service = "nginx";
      defaultLogPathExpr = ''"$NGINX_LOG_FILE"'';
    };
    statusBody = observability.mkStatusLine {
      service = "nginx";
      afterPidFields = [
        "http_port=$HTTP_PORT"
        "https_port=$HTTPS_PORT"
      ];
    };
    healthBody = serviceScripts.mkSimpleProbeBody {
      probeCommand = probeCommands.tcpOpenCmd { portExpr = "$HTTP_PORT"; };
      successMessage = "nginx healthy http_port=$HTTP_PORT";
      failureMessage = "nginx unhealthy http_port=$HTTP_PORT";
    };
    readyBody = serviceScripts.mkSimpleProbeBody {
      probeCommand = probeCommands.tcpOpenCmd { portExpr = "$HTTP_PORT"; };
      successBody = ''
        PID=$(cat "$NGINX_PID_FILE" 2>/dev/null || true)
        emit_service_event service_ready ready --pid "$PID" --log-path "$NGINX_LOG_FILE"
      '';
      successMessage = "nginx ready http_port=$HTTP_PORT pid=\${PID:-unknown}";
      failureMessage = "nginx not ready http_port=$HTTP_PORT";
    };
    stopWaitAttempts = 40;
    stopWaitInterval = "0.25";
  };

  inherit (managedLifecycle)
    init
    start
    stop
    restart
    status
    checkConfig
    health
    ready
    fullStart
    fullStartTest
    ;

  reload = serviceScripts.mkWrappedScript {
    name = "nginx-reload";
    inherit
      loggingPrelude
      runtimePrelude
      ;
    body = ''
      CONF="$NGINX_DIR/conf/nginx.conf"

      if [ ! -f "$CONF" ]; then
        log_error "Nginx not initialized."
        exit 1
      fi

      # Test config before reload
      log_info "Testing nginx configuration"
      ${nginx}/bin/nginx -c "$CONF" -t 2>&1

      log_info "Reloading nginx"
      ${nginx}/bin/nginx -c "$CONF" -s reload
      log_ok "Nginx reloaded"
    '';
  };

  listInstances = serviceScripts.mkWrappedScript {
    name = "nginx-list-instances";
    inherit loggingPrelude;
    runtimePrelude = "";
    body = ''
      echo "Nginx instances:"
      echo ""
      for pidfile in $(find "''${XDG_DATA_HOME:-$HOME/.local/share}" -name "nginx.pid" 2>/dev/null || true); do
        DIR=$(dirname "$(dirname "$pidfile")")
        PID=$(cat "$pidfile" 2>/dev/null || echo "unknown")
        if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
          STATUS="running"
        else
          STATUS="stale"
        fi
        echo "  $DIR (PID: $PID, Status: $STATUS)"
      done
    '';
  };

in
{
  inherit
    nginx
    reload
    generateSelfSignedCert
    listInstances
    ;
  inherit (managedLifecycle)
    init
    start
    stop
    restart
    status
    health
    checkConfig
    ready
    fullStart
    fullStartTest
    ;
}
