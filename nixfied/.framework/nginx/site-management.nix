# Nginx site management - CRUD for sites
{
  pkgs,
  project,
  slots,
  templates,
  lifecycle,
  loggingPrelude,
}:

let
  cfg = project.modules.nginx or { };
  portVarHttp = slots.portVarName (cfg.portKeyHttp or "http");
  portVarHttps = slots.portVarName (cfg.portKeyHttps or "https");
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;
  siteHelpers = ''
    validate_domain() {
      local value="$1"
      if ! echo "$value" | ${pkgs.gnugrep}/bin/grep -Eq '^[A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$'; then
        log_error "invalid domain value=$value"
        return 1
      fi
      if echo "$value" | ${pkgs.gnugrep}/bin/grep -Eq '(^-|-$|[.]{2,})'; then
        log_error "invalid domain value=$value"
        return 1
      fi
      return 0
    }

    validate_upstream_host() {
      local value="$1"
      if echo "$value" | ${pkgs.gnugrep}/bin/grep -Eq '^([A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?|([0-9]{1,3}[.]){3}[0-9]{1,3}|localhost)$'; then
        return 0
      fi
      log_error "invalid upstream host value=$value"
      return 1
    }

    validate_port() {
      local value="$1"
      case "$value" in
        *[!0-9]*|"")
          log_error "upstream port must be numeric value=$value"
          return 1
          ;;
      esac
      if [ "$value" -lt 1 ] || [ "$value" -gt 65535 ]; then
        log_error "upstream port out of range value=$value"
        return 1
      fi
      return 0
    }

    validate_site_root() {
      local value="$1"
      if [ -z "$value" ]; then
        log_error "site root cannot be empty"
        return 1
      fi
      case "$value" in
        /*) ;;
        *)
          log_error "site root must be absolute path value=$value"
          return 1
          ;;
      esac
      if echo "$value" | ${pkgs.gnugrep}/bin/grep -Eq '(^|/)[.]{2}(/|$)'; then
        log_error "site root cannot contain '..' segments value=$value"
        return 1
      fi
      return 0
    }

    escape_sed_replacement() {
      printf '%s' "$1" | ${pkgs.gnused}/bin/sed -e 's/[|&\\]/\\&/g'
    }
  '';

  writeProxySite = pkgs.writeShellScript "nginx-site-proxy" ''
    ${loggingPrelude}

    set -euo pipefail
    ${siteHelpers}
    if [ $# -lt 3 ]; then
      echo "Usage: nginx-site-proxy <domain> <upstream_host> <upstream_port>" >&2
      exit 1
    fi

    DOMAIN="$1"
    UPSTREAM_HOST="$2"
    UPSTREAM_PORT="$3"
    validate_domain "$DOMAIN"
    validate_upstream_host "$UPSTREAM_HOST"
    validate_port "$UPSTREAM_PORT"

    source <(${slots.getSlotInfo})
    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"

    ${pkgs.gnused}/bin/sed \
      -e "s|NGINX_DIR|$(escape_sed_replacement "$NGINX_DIR")|g" \
      -e "s|HTTP_PORT|$(escape_sed_replacement "$HTTP_PORT")|g" \
      -e "s|HTTPS_PORT|$(escape_sed_replacement "$HTTPS_PORT")|g" \
      -e "s|SITE_DOMAIN|$(escape_sed_replacement "$DOMAIN")|g" \
      -e "s|UPSTREAM_HOST|$(escape_sed_replacement "$UPSTREAM_HOST")|g" \
      -e "s|UPSTREAM_PORT|$(escape_sed_replacement "$UPSTREAM_PORT")|g" \
      "${templates.siteProxyTemplate}" > "$CONF"

    ln -sf "$CONF" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    ${lifecycle.generateSelfSignedCert} "$DOMAIN" "$NGINX_DIR/ssl"
  '';

  writeStaticSite = pkgs.writeShellScript "nginx-site-static" ''
    ${loggingPrelude}

    set -euo pipefail
    ${siteHelpers}
    if [ $# -lt 2 ]; then
      echo "Usage: nginx-site-static <domain> <site_root>" >&2
      exit 1
    fi

    DOMAIN="$1"
    SITE_ROOT="$2"
    validate_domain "$DOMAIN"
    validate_site_root "$SITE_ROOT"

    source <(${slots.getSlotInfo})
    HTTP_PORT_VAR="${portVarHttp}"
    HTTPS_PORT_VAR="${portVarHttps}"
    HTTP_PORT="''${!HTTP_PORT_VAR}"
    HTTPS_PORT="''${!HTTPS_PORT_VAR}"

    NGINX_DIR="${nginxDirExpr}"
    CONF="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"

    ${pkgs.gnused}/bin/sed \
      -e "s|NGINX_DIR|$(escape_sed_replacement "$NGINX_DIR")|g" \
      -e "s|HTTP_PORT|$(escape_sed_replacement "$HTTP_PORT")|g" \
      -e "s|HTTPS_PORT|$(escape_sed_replacement "$HTTPS_PORT")|g" \
      -e "s|SITE_DOMAIN|$(escape_sed_replacement "$DOMAIN")|g" \
      -e "s|SITE_ROOT|$(escape_sed_replacement "$SITE_ROOT")|g" \
      "${templates.siteStaticTemplate}" > "$CONF"

    ln -sf "$CONF" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    ${lifecycle.generateSelfSignedCert} "$DOMAIN" "$NGINX_DIR/ssl"
  '';

  addSite = writeProxySite;

  removeSite = pkgs.writeShellScript "nginx-site-remove" ''
    ${loggingPrelude}

    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-remove <domain>" >&2
      exit 1
    fi

    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"

    rm -f "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    rm -f "$NGINX_DIR/conf/sites-available/$DOMAIN.conf"
    log_ok "Site $DOMAIN removed"
  '';

  enableSite = pkgs.writeShellScript "nginx-site-enable" ''
    ${loggingPrelude}

    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-enable <domain>" >&2
      exit 1
    fi

    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"

    AVAIL="$NGINX_DIR/conf/sites-available/$DOMAIN.conf"
    if [ ! -f "$AVAIL" ]; then
      log_error "Site not found: $DOMAIN"
      exit 1
    fi

    ln -sf "$AVAIL" "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    log_ok "Site $DOMAIN enabled"
  '';

  disableSite = pkgs.writeShellScript "nginx-site-disable" ''
    ${loggingPrelude}

    set -euo pipefail
    DOMAIN="''${1:-}"
    if [ -z "$DOMAIN" ]; then
      echo "Usage: nginx-site-disable <domain>" >&2
      exit 1
    fi

    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"

    rm -f "$NGINX_DIR/conf/sites-enabled/$DOMAIN.conf"
    log_ok "Site $DOMAIN disabled"
  '';

  listSites = pkgs.writeShellScript "nginx-site-list" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})
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
