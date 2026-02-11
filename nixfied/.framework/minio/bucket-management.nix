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
    bucketDelete
    bucketList
    policyApply
    ;
}
