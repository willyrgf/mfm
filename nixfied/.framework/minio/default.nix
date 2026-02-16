# MinIO module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  serviceApi = import ../lib/service-api.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  config = import ./config.nix { inherit project; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };

  logs = observability.mkLogScript "minio";
  log = logs;

  events = observability.mkEventsScript "minio";
  logEventExtensions = observability.mkLogEventExtensions {
    service = "minio";
    summaryName = "MinIO";
    logScript = log;
    eventsScript = events;
  };
  bucketMgmt = import ./bucket-management.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };

  publicApi = serviceApi.mkServiceApiV2 {
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
    operations = {
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
      full-start = {
        script = lifecycle.fullStart;
        hook = "FULL_START";
        summary = "Init/check/start MinIO";
        details = "Performs init + check-config + start for MinIO.";
      };
      full-start-test = {
        script = lifecycle.fullStartTest;
        hook = "FULL_START_TEST";
        summary = "Init/check/start MinIO for test profile";
        details = "Performs init + check-config + start for MinIO (test profile).";
      };
      ready = {
        script = lifecycle.ready;
        hook = "READY";
        summary = "Wait for MinIO readiness";
        details = "Checks MinIO readiness endpoint on the configured API port.";
      };
      export-s3-env = {
        script = lifecycle.exportS3Env;
        hook = "EXPORT_S3_ENV";
        summary = "Export S3-compatible environment from MinIO";
        details = "Prints shell exports for AWS/S3 variables targeting the current MinIO endpoint.";
        usage = [ "eval \"$(nix run .#service::minio::export-s3-env -- <bucket> <prefix> <region>)\"" ];
      };
      bucket-ensure = {
        script = bucketMgmt.bucketEnsure;
        hook = "BUCKET_ENSURE";
        summary = "Ensure MinIO bucket exists";
        details = "Creates bucket when missing and succeeds when already present.";
        usage = [ "nix run .#service::minio::bucket-ensure -- <bucket>" ];
      };
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
    }
    // logEventExtensions;
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
    ready
    fullStart
    fullStartTest
    exportS3Env
    ;

  # Bucket management
  inherit (bucketMgmt)
    bucketCreate
    bucketEnsure
    bucketDelete
    bucketList
    policyApply
    ;

  inherit
    log
    logs
    events
    ;

  # Service API contract
  inherit publicApi;
}
