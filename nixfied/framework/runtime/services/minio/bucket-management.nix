# MinIO bucket management
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  runtimeDefaults = import ../../../core/runtime-defaults.nix;
  mc = config.clientPackage or pkgs.minio-client;
  apiPortVar = slots.portVarName config.portKeyApi;
  minioDirExpr = slots.getServiceDir config.dataDirName;
  minioEndpointExpr = "http://${runtimeDefaults.hosts.loopbackIp}:$MINIO_API_PORT";

  mkBucketScript =
    name: body:
    pkgs.writeShellScript name ''
      ${loggingPrelude}

      set -euo pipefail
      ${body}
    '';

  bucketRuntimePrelude = ''
    source <(${slots.getSlotInfo})
    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"
  '';

  mcAliasSetup = ''
    export MC_CONFIG_DIR="$MINIO_DIR/config/mc"
    ${mc}/bin/mc alias set local "${minioEndpointExpr}" "$ROOT_USER" "$ROOT_PASSWORD" >/dev/null
  '';

  requireBucketArg = commandName: ''
    BUCKET="''${1:-}"
    if [ -z "$BUCKET" ]; then
      nixfied_exit_usage "usage: ${commandName} <bucket>"
    fi
  '';

  bucketCreate = mkBucketScript "minio-bucket-create" ''
    ${requireBucketArg "minio-bucket-create"}
    ${bucketRuntimePrelude}
    ${mcAliasSetup}
    ${mc}/bin/mc mb --ignore-existing "local/$BUCKET"

    log_ok "minio bucket created bucket=$BUCKET"
  '';

  bucketEnsure = mkBucketScript "minio-bucket-ensure" ''
    ${requireBucketArg "minio-bucket-ensure"}
    ${bucketRuntimePrelude}
    ${mcAliasSetup}
    ${mc}/bin/mc mb --ignore-existing "local/$BUCKET" >/dev/null

    log_ok "minio bucket ensured bucket=$BUCKET"
  '';

  bucketDelete = mkBucketScript "minio-bucket-delete" ''
    ${requireBucketArg "minio-bucket-delete"}
    ${bucketRuntimePrelude}
    ${mcAliasSetup}
    ${mc}/bin/mc rb --force "local/$BUCKET"

    log_ok "minio bucket deleted bucket=$BUCKET"
  '';

  bucketList = mkBucketScript "minio-bucket-list" ''
    ${bucketRuntimePrelude}
    ${mcAliasSetup}
    ${mc}/bin/mc ls local
  '';

  policyApply = mkBucketScript "minio-policy-apply" ''
    BUCKET="''${1:-}"
    POLICY_FILE="''${2:-}"

    if [ -z "$BUCKET" ] || [ -z "$POLICY_FILE" ]; then
      nixfied_exit_usage "usage: minio-policy-apply <bucket> <policy-file>"
    fi

    if [ ! -f "$POLICY_FILE" ]; then
      nixfied_exit_precondition "policy file not found: $POLICY_FILE"
    fi

    ${bucketRuntimePrelude}
    ${mcAliasSetup}
    ${mc}/bin/mc anonymous set-json "$POLICY_FILE" "local/$BUCKET"

    log_ok "minio policy applied bucket=$BUCKET file=$POLICY_FILE"
  '';
in
{
  inherit
    bucketCreate
    bucketEnsure
    bucketDelete
    bucketList
    policyApply
    ;
}
