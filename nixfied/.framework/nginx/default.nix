# Nginx module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.nginx or { };
  portVarHttp = slots.portVarName (cfg.portKeyHttp or "http");
  portVarHttps = slots.portVarName (cfg.portKeyHttps or "https");
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;

  templates = import ./templates.nix { inherit pkgs; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      templates
      ;
  };
  siteMgmt = import ./site-management.nix {
    inherit
      pkgs
      project
      slots
      templates
      lifecycle
      ;
  };
  ssl = import ./ssl.nix {
    inherit
      pkgs
      project
      slots
      lifecycle
      ;
  };

  restart = pkgs.writeShellScript "nginx-restart" ''
    set -euo pipefail

    ${lifecycle.stop}
    exec ${lifecycle.start}
  '';

  status = pkgs.writeShellScript "nginx-status" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    NGINX_DIR="${nginxDirExpr}"
    PID_FILE="$NGINX_DIR/run/nginx.pid"
    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"

    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    RUNNING=false
    PID=""

    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    echo "service=nginx slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} http_port=$HTTP_PORT https_port=$HTTPS_PORT"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "nginx-health" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    HTTP_PORT_VAR="${portVarHttp}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"

    if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$HTTP_PORT" >/dev/null 2>&1; then
      echo "OK: nginx healthy http_port=$HTTP_PORT"
      exit 0
    fi

    echo "ERROR: nginx unhealthy http_port=$HTTP_PORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "nginx-check-config" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/nginx.conf"

    if [ ! -f "$CONF" ]; then
      echo "ERROR: missing nginx config at $CONF" >&2
      exit 1
    fi

    ${lifecycle.nginx}/bin/nginx -c "$CONF" -t 2>&1
    echo "OK: nginx configuration valid conf=$CONF"
  '';

  publicApi = {
    version = 1;
    service = "nginx";
    summary = "Nginx service management API";
    details = "Public service contract for managing nginx across dev/prod/test/ci.";
    profiles = [
      "dev"
      "prod"
      "test"
      "ci"
    ];
    artifacts = {
      httpPortVar = portVarHttp;
      httpsPortVar = portVarHttps;
      dataDir = nginxDirExpr;
    };
    coreOps = {
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
    };
    extensions = {
      reload = {
        script = lifecycle.reload;
        summary = "Reload nginx configuration";
        details = "Tests and reloads nginx configuration.";
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
        app = false;
        usage = [ "nix run .#service::nginx::site-proxy -- <domain> <upstream-host> <upstream-port>" ];
      };
      site-static = {
        script = siteMgmt.writeStaticSite;
        hook = "SITE_STATIC";
        summary = "Write static site configuration";
        details = "Writes a static site config and enables it.";
        app = false;
        usage = [ "nix run .#service::nginx::site-static -- <domain> <site-root>" ];
      };
      site-add = {
        script = siteMgmt.addSite;
        hook = "SITE_ADD";
        summary = "Add proxy nginx site";
        details = "Adds a proxy site and enables it.";
        usage = [ "nix run .#service::nginx::site-add -- <domain> <upstream-host> <upstream-port>" ];
      };
      site-remove = {
        script = siteMgmt.removeSite;
        hook = "SITE_REMOVE";
        summary = "Remove nginx site";
        details = "Removes nginx site configuration.";
        usage = [ "nix run .#service::nginx::site-remove -- <domain>" ];
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
        usage = [ "nix run .#service::nginx::site-enable -- <domain>" ];
      };
      site-disable = {
        script = siteMgmt.disableSite;
        hook = "SITE_DISABLE";
        summary = "Disable nginx site";
        details = "Disables an existing nginx site.";
        usage = [ "nix run .#service::nginx::site-disable -- <domain>" ];
      };
      cert-obtain = {
        script = ssl.obtainCert;
        hook = "CERT_OBTAIN";
        summary = "Obtain SSL certificate";
        details = "Obtains a Let's Encrypt certificate for a domain.";
        usage = [ "nix run .#service::nginx::cert-obtain -- <domain> <email> [--staging]" ];
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
    };
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
    checkConfig
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
