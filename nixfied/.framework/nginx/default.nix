# Nginx module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.nginx or { };
  summary = import ../lib/summary.nix { inherit pkgs project; };
  helpers = import ../lib/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  slotEnvRuntime = import ../lib/slot-env-runtime.nix { inherit pkgs; };
  serviceApi = import ../lib/service-api.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  portVarHttp = slots.portVarName (cfg.portKeyHttp or "http");
  portVarHttps = slots.portVarName (cfg.portKeyHttps or "https");
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;
  runtimePrimitives = serviceApi.mkRuntimePrimitivesV1 {
    logLevelDefault = toString ((project.logging or { }).level or "info");
    outputModeDefault = toString ((project.logging or { }).output or "stdout");
  };
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

    if [ -z "$HTTP_PORT" ] || [ -z "$HTTPS_PORT" ]; then
      log_error "nginx port variables are not set (http/https)"
      exit 1
    fi
  '';

  templates = import ./templates.nix { inherit pkgs; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      templates
      loggingPrelude
      ;
  };
  siteMgmt = import ./site-management.nix {
    inherit
      pkgs
      project
      slots
      templates
      lifecycle
      loggingPrelude
      ;
  };
  ssl = import ./ssl.nix {
    inherit
      pkgs
      project
      slots
      lifecycle
      loggingPrelude
      ;
  };

  restart = pkgs.writeShellScript "nginx-restart" ''
    set -euo pipefail

    ${lifecycle.stop}
    exec ${lifecycle.start}
  '';

  status = pkgs.writeShellScript "nginx-status" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}
    PID_FILE="$NGINX_DIR/run/nginx.pid"

    RUNNING=false
    PID=""

    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    ${observability.mkStatusMergeBlock {
      service = "nginx";
      defaultLogPathExpr = ''"$NGINX_DIR/logs/error.log"'';
    }}

    echo "service=nginx slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} http_port=$HTTP_PORT https_port=$HTTPS_PORT scope=$SCOPE owner_run_id=''${OWNER_RUN_ID:-unknown} owner_scope=''${OWNER_SCOPE:-unknown} ephemeral_root=''${EPHEMERAL_ROOT:-none} registry_state=''${REGISTRY_STATE:-unknown} slot_owner=''${SLOT_OWNER:-unknown} wait_reason=''${WAIT_REASON:-none} log_path=$EFFECTIVE_LOG_PATH"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "nginx-health" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}
    PID_FILE="$NGINX_DIR/run/nginx.pid"

    PID=""
    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
    fi

    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
      if ${pkgs.netcat}/bin/nc -z 127.0.0.1 "$HTTP_PORT" >/dev/null 2>&1; then
        log_ok "nginx healthy http_port=$HTTP_PORT pid=$PID"
        exit 0
      fi
    fi

    log_error "nginx unhealthy http_port=$HTTP_PORT pid=''${PID:-unknown}"
    exit 1
  '';

  ready = pkgs.writeShellScript "nginx-ready" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}
    PID_FILE="$NGINX_DIR/run/nginx.pid"

    PID=""
    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
    fi

    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      log_error "nginx not ready (process not running) http_port=$HTTP_PORT pid=''${PID:-unknown}"
      exit 1
    fi

    if ${pkgs.netcat}/bin/nc -z 127.0.0.1 "$HTTP_PORT" >/dev/null 2>&1; then
      ${processRegistry.emitEvent} \
        --event-type service_ready \
        --service nginx \
        --state ready \
        --slot "$SLOT" \
        --env "$ENV" \
        --pid "$PID" \
        --log-path "$NGINX_DIR/logs/error.log" >/dev/null 2>&1 || true
      log_ok "nginx ready http_port=$HTTP_PORT pid=$PID"
      exit 0
    fi

    log_error "nginx not ready http_port=$HTTP_PORT pid=$PID"
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "nginx-check-config" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}
    CONF="$NGINX_DIR/conf/nginx.conf"

    if [ ! -f "$CONF" ]; then
      log_error "missing nginx config at $CONF"
      exit 1
    fi

    ${lifecycle.nginx}/bin/nginx -c "$CONF" -t 2>&1
    log_ok "nginx configuration valid conf=$CONF"
  '';

  logs = observability.mkLogScript "nginx";
  log = logs;

  events = observability.mkEventsScript "nginx";
  logEventExtensions = observability.mkLogEventExtensions {
    service = "nginx";
    summaryName = "nginx";
    logScript = log;
    eventsScript = events;
  };

  publicApi = serviceApi.mkServiceApiV3 {
    service = "nginx";
    summary = "Nginx service management API";
    details = "Public service contract for managing nginx across dev/prod/test/ci.";
    inherit runtimePrimitives;
    artifacts = {
      httpPortVar = portVarHttp;
      httpsPortVar = portVarHttps;
      dataDir = nginxDirExpr;
    };
    operations = {
      init = {
        script = lifecycle.init;
        summary = "Initialize nginx directories and config";
        details = "Creates nginx runtime directories and base configuration.";
      };
      start = {
        script = lifecycle.start;
        summary = "Start nginx server";
        details = "Starts nginx for the current slot and environment.";
      };
      stop = {
        script = lifecycle.stop;
        summary = "Stop nginx server";
        details = "Stops nginx for the current slot and environment.";
      };
      restart = {
        script = restart;
        summary = "Restart nginx server";
        details = "Stops then starts nginx for the current slot and environment.";
      };
      status = {
        script = status;
        summary = "Show nginx status";
        details = "Prints nginx status for the current slot and environment.";
      };
      health = {
        script = health;
        summary = "Run nginx health check";
        details = "Checks that nginx responds on the configured HTTP port.";
      };
      check-config = {
        script = checkConfig;
        summary = "Validate nginx configuration";
        details = "Runs nginx config validation for the current slot and environment.";
      };
    }
    // {
      reload = {
        script = lifecycle.reload;
        summary = "Reload nginx configuration";
        details = "Tests and reloads nginx configuration.";
      };
      ready = {
        script = ready;
        hook = "READY";
        summary = "Wait for nginx readiness";
        details = "Checks that nginx serves HTTP requests on the configured port.";
      };
      list-instances = {
        script = lifecycle.listInstances;
        hook = "LIST_INSTANCES";
        summary = "List nginx instances";
        details = "Lists nginx instances managed by Nixfied.";
      };
      site-proxy = {
        script = siteMgmt.writeProxySite;
        hook = "SITE_PROXY";
        summary = "Write proxy site configuration";
        details = "Writes a proxy site config and enables it.";
        exposeApp = false;
        usage = [ "nix run .#svc::nginx::site-proxy -- <domain> <upstream-host> <upstream-port>" ];
      };
      site-static = {
        script = siteMgmt.writeStaticSite;
        hook = "SITE_STATIC";
        summary = "Write static site configuration";
        details = "Writes a static site config and enables it.";
        exposeApp = false;
        usage = [ "nix run .#svc::nginx::site-static -- <domain> <site-root>" ];
      };
      site-add = {
        script = siteMgmt.addSite;
        hook = "SITE_ADD";
        summary = "Add proxy nginx site";
        details = "Adds a proxy site and enables it.";
        usage = [ "nix run .#svc::nginx::site-add -- <domain> <upstream-host> <upstream-port>" ];
      };
      site-remove = {
        script = siteMgmt.removeSite;
        hook = "SITE_REMOVE";
        summary = "Remove nginx site";
        details = "Removes nginx site configuration.";
        usage = [ "nix run .#svc::nginx::site-remove -- <domain>" ];
      };
      site-list = {
        script = siteMgmt.listSites;
        hook = "SITE_LIST";
        summary = "List nginx sites";
        details = "Lists configured nginx sites.";
      };
      site-enable = {
        script = siteMgmt.enableSite;
        hook = "SITE_ENABLE";
        summary = "Enable nginx site";
        details = "Enables an existing nginx site.";
        usage = [ "nix run .#svc::nginx::site-enable -- <domain>" ];
      };
      site-disable = {
        script = siteMgmt.disableSite;
        hook = "SITE_DISABLE";
        summary = "Disable nginx site";
        details = "Disables an existing nginx site.";
        usage = [ "nix run .#svc::nginx::site-disable -- <domain>" ];
      };
      cert-obtain = {
        script = ssl.obtainCert;
        hook = "CERT_OBTAIN";
        summary = "Obtain SSL certificate";
        details = "Obtains a Let's Encrypt certificate for a domain.";
        usage = [ "nix run .#svc::nginx::cert-obtain -- <domain> <email> [--staging]" ];
      };
      cert-renew = {
        script = ssl.renewCerts;
        hook = "CERT_RENEW";
        summary = "Renew SSL certificates";
        details = "Renews certificates for configured sites.";
      };
      cert-status = {
        script = ssl.certStatus;
        hook = "CERT_STATUS";
        summary = "Show SSL certificate status";
        details = "Prints certificate status for configured domains.";
      };
    }
    // logEventExtensions;
  };
in
{
  # Lifecycle (backward compat)
  inherit (lifecycle)
    nginx
    init
    start
    stop
    reload
    generateSelfSignedCert
    listInstances
    ;

  # Extended lifecycle
  inherit
    restart
    status
    health
    ready
    checkConfig
    log
    logs
    events
    ;

  # Site management (backward compat + new)
  inherit (siteMgmt)
    writeProxySite
    writeStaticSite
    addSite
    removeSite
    enableSite
    disableSite
    listSites
    ;

  # SSL
  inherit (ssl) obtainCert renewCerts certStatus;

  # Service API contract
  inherit publicApi;
}
