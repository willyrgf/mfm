# Nginx site management - CRUD for sites
{
  pkgs,
  project,
  slots,
  templates,
  lifecycle,
}:

let
  cfg = project.modules.nginx or { };
  portVarHttp = slots.portVarName (cfg.portKeyHttp or "http");
  portVarHttps = slots.portVarName (cfg.portKeyHttps or "https");
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;

  writeProxySite = pkgs.writeShellScript "nginx-site-proxy" ''
    set -euo pipefail
    if [ $# -lt 3 ]; then
      echo "Usage: nginx-site-proxy <domain> <upstream_host> <upstream_port>" >&2
      exit 1
    fi

    DOMAIN="$1"
    UPSTREAM_HOST="$2"
    UPSTREAM_PORT="$3"

    eval "$(${slots.getSlotInfo})"
    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"

    ${pkgs.gnused}/bin/sed \
      -e "s|NGINX_DIR|$NGINX_DIR|g" \
      -e "s|HTTP_PORT|$HTTP_PORT|g" \
      -e "s|HTTPS_PORT|$HTTPS_PORT|g" \
      -e "s|SITE_DOMAIN|$DOMAIN|g" \
      -e "s|UPSTREAM_HOST|$UPSTREAM_HOST|g" \
      -e "s|UPSTREAM_PORT|$UPSTREAM_PORT|g" \
      "${templates.siteProxyTemplate}" > "$CONF"

    ln -sf "$CONF" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    ${lifecycle.generateSelfSignedCert} "$DOMAIN" "$NGINX_DIR/ssl"
  '';

  writeStaticSite = pkgs.writeShellScript "nginx-site-static" ''
    set -euo pipefail
    if [ $# -lt 2 ]; then
      echo "Usage: nginx-site-static <domain> <site_root>" >&2
      exit 1
    fi

    DOMAIN="$1"
    SITE_ROOT="$2"

    eval "$(${slots.getSlotInfo})"
    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"

    ${pkgs.gnused}/bin/sed \
      -e "s|NGINX_DIR|$NGINX_DIR|g" \
      -e "s|HTTP_PORT|$HTTP_PORT|g" \
      -e "s|HTTPS_PORT|$HTTPS_PORT|g" \
      -e "s|SITE_DOMAIN|$DOMAIN|g" \
      -e "s|SITE_ROOT|$SITE_ROOT|g" \
      "${templates.siteStaticTemplate}" > "$CONF"

    ln -sf "$CONF" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    ${lifecycle.generateSelfSignedCert} "$DOMAIN" "$NGINX_DIR/ssl"
  '';

  addSite = writeProxySite;

  removeSite = pkgs.writeShellScript "nginx-site-remove" ''
    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-remove <domain>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"

    rm -f "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    rm -f "$NGINX_DIR/conf/sites-available/$DOMAIN.conf"
    echo "✅ Site $DOMAIN removed"
  '';

  enableSite = pkgs.writeShellScript "nginx-site-enable" ''
    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-enable <domain>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"

    AVAIL="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"
    if [ ! -f "$AVAIL" ]; then
      echo "❌ Site not found: $DOMAIN" >&2
      exit 1
    fi

    ln -sf "$AVAIL" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    echo "✅ Site $DOMAIN enabled"
  '';

  disableSite = pkgs.writeShellScript "nginx-site-disable" ''
    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-disable <domain>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"

    rm -f "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    echo "✅ Site $DOMAIN disabled"
  '';

  listSites = pkgs.writeShellScript "nginx-site-list" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    NGINX_DIR="${nginxDirExpr}"

    AVAIL_DIR="$NGINX_DIR/conf/sites-available"
    ENABLED_DIR="$NGINX_DIR/conf/sites-enabled"

    if [ ! -d "$AVAIL_DIR" ]; then
      echo "No sites configured"
      exit 0
    fi

    echo "Sites (slot $SLOT, env $ENV):"
    echo ""
    for conf in "$AVAIL_DIR"/*.conf; do
      [ -f "$conf" ] || continue
      DOMAIN=$(basename "$conf" .conf)
      ENABLED="disabled"
      if [ -L "$ENABLED_DIR/$DOMAIN.conf" ] || [ -f "$ENABLED_DIR/$DOMAIN.conf" ]; then
        ENABLED="enabled"
      fi

      # Check SSL type
      CERT_DIR="$NGINX_DIR/ssl/live/$DOMAIN"
      SSL_TYPE="none"
      EXPIRY="n/a"
      if [ -f "$CERT_DIR/fullchain.pem" ]; then
        ISSUER=$(${pkgs.openssl}/bin/openssl x509 -in "$CERT_DIR/fullchain.pem" -issuer -noout 2>/dev/null || echo "unknown")
        if echo "$ISSUER" | grep -qiE "encrypt|R3|E1"; then
          SSL_TYPE="letsencrypt"
        else
          SSL_TYPE="self-signed"
        fi
        EXPIRY=$(${pkgs.openssl}/bin/openssl x509 -in "$CERT_DIR/fullchain.pem" -enddate -noout 2>/dev/null | cut -d= -f2 || echo "unknown")
      fi

      echo "  $DOMAIN [$ENABLED] SSL: $SSL_TYPE Expires: $EXPIRY"
    done
  '';

in
{
  inherit
    writeProxySite
    writeStaticSite
    addSite
    removeSite
    enableSite
    disableSite
    listSites
    ;
}
