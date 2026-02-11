# MinIO module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  config = import ./config.nix { inherit project; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };
  bucketMgmt = import ./bucket-management.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };

  publicApi = {
    version = 1;
    service = "minio";
    summary = "MinIO service management API";
    details = "Public service contract for managing MinIO across dev/prod/test/ci.";
    profiles = [
      "dev"
      "prod"
      "test"
      "ci"
    ];
    artifacts = {
      apiPortVar = slots.portVarName config.portKeyApi;
      consolePortVar = slots.portVarName config.portKeyConsole;
      dataDir = slots.getServiceDir config.dataDirName;
    };
    coreOps = {
      init = {
        script = lifecycle.init;
        summary = "Initialize MinIO data directories";
        details = "Creates MinIO runtime directories for the current slot and environment.";
      };
      start = {
        script = lifecycle.start;
        summary = "Start MinIO server";
        details = "Starts MinIO with API and console listeners for the current slot and environment.";
      };
      stop = {
        script = lifecycle.stop;
        summary = "Stop MinIO server";
        details = "Stops MinIO for the current slot and environment.";
      };
      restart = {
        script = lifecycle.restart;
        summary = "Restart MinIO server";
        details = "Stops then starts MinIO for the current slot and environment.";
      };
      status = {
        script = lifecycle.status;
        summary = "Show MinIO status";
        details = "Prints MinIO status for the current slot and environment.";
      };
      health = {
        script = lifecycle.health;
        summary = "Run MinIO health check";
        details = "Checks MinIO health endpoint for the configured API port.";
      };
      check-config = {
        script = lifecycle.checkConfig;
        summary = "Validate MinIO configuration";
        details = "Validates MinIO binary and runtime configuration directories.";
      };
    };
    extensions = {
      bucket-create = {
        script = bucketMgmt.bucketCreate;
        hook = "BUCKET_CREATE";
        summary = "Create MinIO bucket";
        details = "Creates a bucket in the running MinIO instance.";
        usage = [ "nix run .#service::minio::bucket-create -- <bucket>" ];
      };
      bucket-delete = {
        script = bucketMgmt.bucketDelete;
        hook = "BUCKET_DELETE";
        summary = "Delete MinIO bucket";
        details = "Deletes a bucket from the running MinIO instance.";
        usage = [ "nix run .#service::minio::bucket-delete -- <bucket>" ];
      };
      bucket-list = {
        script = bucketMgmt.bucketList;
        hook = "BUCKET_LIST";
        summary = "List MinIO buckets";
        details = "Lists buckets from the running MinIO instance.";
      };
      policy-apply = {
        script = bucketMgmt.policyApply;
        hook = "POLICY_APPLY";
        summary = "Apply MinIO bucket policy";
        details = "Applies a JSON policy file to a MinIO bucket.";
        usage = [ "nix run .#service::minio::policy-apply -- <bucket> <policy-file>" ];
      };
    };
  };
in
{
  inherit config;

  # Lifecycle
  inherit (lifecycle)
    minio
    init
    start
    stop
    restart
    status
    health
    checkConfig
    ;

  # Bucket management
  inherit (bucketMgmt)
    bucketCreate
    bucketDelete
    bucketList
    policyApply
    ;

  # Service API contract
  inherit publicApi;
}
