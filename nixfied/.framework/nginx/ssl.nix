# Nginx SSL management - Let's Encrypt / certbot integration
{
  pkgs,
  project,
  slots,
  lifecycle,
  loggingPrelude,
}:

let
  cfg = project.modules.nginx or { };
  dataDirName = cfg.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;

  obtainCert = pkgs.writeShellScript "nginx-cert-obtain" ''
    ${loggingPrelude}

    set -euo pipefail

    DOMAIN="''${1:-}"
    EMAIL="''${2:-}"
    STAGING="''${3:-}"

    if [ -z "$DOMAIN" ] || [ -z "$EMAIL" ]; then
      echo "Usage: nginx-cert-obtain <domain> <email> [--staging]" >&2
      exit 1
    fi

    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"
    WEBROOT="$NGINX_DIR/html"

    STAGING_FLAG=""
    if [ "$STAGING" = "--staging" ]; then
      STAGING_FLAG="--staging"
    fi

    log_info "Obtaining Let's Encrypt certificate for $DOMAIN"

    ${pkgs.certbot}/bin/certbot certonly \
      --webroot \
      --webroot-path "$WEBROOT" \
      --email "$EMAIL" \
      --agree-tos \
      --no-eff-email \
      --cert-path "$NGINX_DIR/ssl/live/$DOMAIN" \
      --key-path "$NGINX_DIR/ssl/live/$DOMAIN" \
      -d "$DOMAIN" \
      $STAGING_FLAG

    log_ok "Certificate obtained for $DOMAIN"
  '';

  renewCerts = pkgs.writeShellScript "nginx-cert-renew" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"

    log_info "Renewing certificates"

    ${pkgs.certbot}/bin/certbot renew \
      --deploy-hook "${lifecycle.reload}" \
      2>&1

    log_ok "Certificate renewal complete"
  '';

  certStatus = pkgs.writeShellScript "nginx-cert-status" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})
    NGINX_DIR="${nginxDirExpr}"

    SSL_DIR="$NGINX_DIR/ssl/live"

    if [ ! -d "$SSL_DIR" ]; then
      echo "No certificates found"
      exit 0
    fi

    echo "SSL Certificates (slot $SLOT, env $ENV):"
    echo ""
    for certdir in "$SSL_DIR"/*/; do
      [ -d "$certdir" ] || continue
      DOMAIN=$(basename "$certdir")
      if [ -f "$certdir/fullchain.pem" ]; then
        EXPIRY=$(${pkgs.openssl}/bin/openssl x509 -in "$certdir/fullchain.pem" -enddate -noout 2>/dev/null | cut -d= -f2 || echo "unknown")
        ISSUER=$(${pkgs.openssl}/bin/openssl x509 -in "$certdir/fullchain.pem" -issuer -noout 2>/dev/null | sed 's/issuer=//' || echo "unknown")
        DAYS_LEFT=$(( ( $(${pkgs.openssl}/bin/openssl x509 -in "$certdir/fullchain.pem" -enddate -noout 2>/dev/null | cut -d= -f2 | xargs -I{} date -d {} +%s 2>/dev/null || date +%s) - $(date +%s) ) / 86400 )) 2>/dev/null || DAYS_LEFT="?"
        echo "  $DOMAIN"
        echo "    Expires: $EXPIRY ($DAYS_LEFT days)"
        echo "    Issuer: $ISSUER"
      else
        echo "  $DOMAIN (no certificate)"
      fi
    done
  '';

in
{
  inherit obtainCert renewCerts certStatus;
}
