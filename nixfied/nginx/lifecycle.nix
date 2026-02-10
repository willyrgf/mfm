# Nginx lifecycle management - init, start, stop, reload
{
  pkgs,
  project,
  slots,
  templates,
}:

let
  cfg = project.modules.nginx or { };
  nginx = templates.nginx;
  portVarHttp = slots.portVarName (cfg.portKeyHttp or "http");
  portVarHttps = slots.portVarName (cfg.portKeyHttps or "https");
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;

  generateSelfSignedCert = pkgs.writeShellScript "nginx-generate-self-signed" ''
    set -euo pipefail
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

  init = pkgs.writeShellScript "nginx-init" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    NGINX_DIR="${nginxDirExpr}"

    mkdir -p "$NGINX_DIR/conf/sites-available"
    mkdir -p "$NGINX_DIR/conf/sites-enabled"
    mkdir -p "$NGINX_DIR/ssl/live/localhost"
    mkdir -p "$NGINX_DIR/logs"
    mkdir -p "$NGINX_DIR/html"
    mkdir -p "$NGINX_DIR/run"

    ${pkgs.gnused}/bin/sed \
      -e "s|NGINX_DIR|$NGINX_DIR|g" \
      -e "s|HTTP_PORT|$HTTP_PORT|g" \
      -e "s|HTTPS_PORT|$HTTPS_PORT|g" \
      "${templates.nginxConfTemplate}" > "$NGINX_DIR/conf/nginx.conf"

    ${generateSelfSignedCert} "localhost" "$NGINX_DIR/ssl"
  '';

  start = pkgs.writeShellScript "nginx-start" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"

    CONF="$NGINX_DIR/conf/nginx.conf"
    if [ ! -f "$CONF" ]; then
      echo "ERROR: Nginx not initialized. Run nginx-init first." >&2
      exit 1
    fi

    ${nginx}/bin/nginx -c "$CONF" -g 'daemon off;'
  '';

  stop = pkgs.writeShellScript "nginx-stop" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"
    PID_FILE="$NGINX_DIR/run/nginx.pid"

    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        ${nginx}/bin/nginx -c "$NGINX_DIR/conf/nginx.conf" -s quit || true
      else
        rm -f "$PID_FILE"
      fi
    fi
  '';

  reload = pkgs.writeShellScript "nginx-reload" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/nginx.conf"

    if [ ! -f "$CONF" ]; then
      echo "ERROR: Nginx not initialized." >&2
      exit 1
    fi

    # Test config before reload
    echo "INFO: Testing nginx configuration"
    ${nginx}/bin/nginx -c "$CONF" -t 2>&1

    echo "INFO: Reloading nginx"
    ${nginx}/bin/nginx -c "$CONF" -s reload
    echo "OK: Nginx reloaded"
  '';

  listInstances = pkgs.writeShellScript "nginx-list-instances" ''
    set -euo pipefail
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

in
{
  inherit
    nginx
    init
    start
    stop
    reload
    generateSelfSignedCert
    listInstances
    ;
}
