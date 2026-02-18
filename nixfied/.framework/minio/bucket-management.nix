# MinIO bucket management
{
  pkgs,
  project,
  slots,
  config,
}:

let
  cfg = project.modules.minio or { };
  mc = cfg.clientPackage or pkgs.minio-client;
  apiPortVar = slots.portVarName config.portKeyApi;
  minioDirExpr = slots.getServiceDir config.dataDirName;

  bucketCreate = pkgs.writeShellScript "minio-bucket-create" ''
    set -euo pipefail

    BUCKET="''${1:-}"
    if [ -z "$BUCKET" ]; then
      echo "usage: minio-bucket-create <bucket>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
    ${mc}/bin/mc mb --ignore-existing "local/$BUCKET"

    echo "OK: minio bucket created bucket=$BUCKET"
  '';

  bucketEnsure = pkgs.writeShellScript "minio-bucket-ensure" ''
    set -euo pipefail

    BUCKET="''${1:-}"
    if [ -z "$BUCKET" ]; then
      echo "usage: minio-bucket-ensure <bucket>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
    ${mc}/bin/mc mb --ignore-existing "local/$BUCKET" >/dev/null

    echo "OK: minio bucket ensured bucket=$BUCKET"
  '';

  bucketProbeWrite = pkgs.writeShellScript "minio-bucket-probe-write" ''
    set -euo pipefail

    BUCKET="''${1:-}"
    PREFIX="''${2:-}"
    if [ -z "$BUCKET" ]; then
      echo "usage: minio-bucket-probe-write <bucket> [prefix]" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
    ${mc}/bin/mc mb --ignore-existing "local/$BUCKET" >/dev/null

    PROBE_SUFFIX="''${RANDOM:-0}-$$-$(date +%s)"
    if [ -n "$PREFIX" ]; then
      PROBE_OBJECT="$PREFIX/__ci_probe__/$PROBE_SUFFIX.txt"
    else
      PROBE_OBJECT="__ci_probe__/$PROBE_SUFFIX.txt"
    fi

    PROBE_TMP="$(mktemp "''${TMPDIR:-/tmp}/minio-write-probe.XXXXXX")"
    printf 'minio-write-probe %s\n' "$(date -u +%FT%TZ)" > "$PROBE_TMP"
    DISK_STATS="$(${pkgs.coreutils}/bin/df -Pk "$MINIO_DIR/data" | tail -n 1 || true)"
    DISK_TOTAL_KB="unknown"
    DISK_USED_KB="unknown"
    DISK_AVAIL_KB="unknown"
    DISK_USED_PCT="unknown"
    if [ -n "$DISK_STATS" ]; then
      read -r _ DISK_TOTAL_KB DISK_USED_KB DISK_AVAIL_KB DISK_USED_PCT _ <<EOF || true
$DISK_STATS
EOF
    fi

    if ! ${mc}/bin/mc cp "$PROBE_TMP" "local/$BUCKET/$PROBE_OBJECT" >/dev/null; then
      rm -f "$PROBE_TMP"
      echo "ERROR: minio write probe failed bucket=$BUCKET object=$PROBE_OBJECT endpoint=http://127.0.0.1:$MINIO_API_PORT disk_total_kb=$DISK_TOTAL_KB disk_used_kb=$DISK_USED_KB disk_avail_kb=$DISK_AVAIL_KB disk_used_pct=$DISK_USED_PCT" >&2
      exit 1
    fi

    if ! ${mc}/bin/mc stat "local/$BUCKET/$PROBE_OBJECT" >/dev/null 2>&1; then
      rm -f "$PROBE_TMP"
      echo "ERROR: minio write probe stat failed bucket=$BUCKET object=$PROBE_OBJECT endpoint=http://127.0.0.1:$MINIO_API_PORT disk_total_kb=$DISK_TOTAL_KB disk_used_kb=$DISK_USED_KB disk_avail_kb=$DISK_AVAIL_KB disk_used_pct=$DISK_USED_PCT" >&2
      exit 1
    fi

    if ! ${mc}/bin/mc rm --force "local/$BUCKET/$PROBE_OBJECT" >/dev/null; then
      rm -f "$PROBE_TMP"
      echo "ERROR: minio write probe cleanup failed bucket=$BUCKET object=$PROBE_OBJECT endpoint=http://127.0.0.1:$MINIO_API_PORT disk_total_kb=$DISK_TOTAL_KB disk_used_kb=$DISK_USED_KB disk_avail_kb=$DISK_AVAIL_KB disk_used_pct=$DISK_USED_PCT" >&2
      exit 1
    fi

    rm -f "$PROBE_TMP"
    echo "OK: minio write probe passed bucket=$BUCKET object=$PROBE_OBJECT disk_avail_kb=$DISK_AVAIL_KB"
  '';

  bucketDelete = pkgs.writeShellScript "minio-bucket-delete" ''
    set -euo pipefail

    BUCKET="''${1:-}"
    if [ -z "$BUCKET" ]; then
      echo "usage: minio-bucket-delete <bucket>" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
    ${mc}/bin/mc rb --force "local/$BUCKET"

    echo "OK: minio bucket deleted bucket=$BUCKET"
  '';

  bucketList = pkgs.writeShellScript "minio-bucket-list" ''
    set -euo pipefail

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null

    ${mc}/bin/mc ls local
  '';

  policyApply = pkgs.writeShellScript "minio-policy-apply" ''
    set -euo pipefail

    BUCKET="''${1:-}"
    POLICY_FILE="''${2:-}"

    if [ -z "$BUCKET" ] || [ -z "$POLICY_FILE" ]; then
      echo "usage: minio-policy-apply <bucket> <policy-file>" >&2
      exit 1
    fi

    if [ ! -f "$POLICY_FILE" ]; then
      echo "ERROR: policy file not found: $POLICY_FILE" >&2
      exit 1
    fi

    eval "$(${slots.getSlotInfo})"
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "http://127.0.0.1:$MINIO_API_PORT" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
    ${mc}/bin/mc anonymous set-json "$POLICY_FILE" "local/$BUCKET"

    echo "OK: minio policy applied bucket=$BUCKET file=$POLICY_FILE"
  '';
in
{
  inherit
    bucketCreate
    bucketEnsure
    bucketProbeWrite
    bucketDelete
    bucketList
    policyApply
    ;
}
