# Module hook registry (optional)
{
  pkgs,
  project,
  slots,
  postgres ? null,
  nginx ? null,
  minio ? null,
  supervisor ? null,
  ephemeral ? null,
  serviceApis ? { },
}:

let
  serviceApi = import ./lib/service-api.nix { inherit pkgs; };
  effectiveServiceApis =
    if serviceApis != { } then
      serviceApis
    else
      (pkgs.lib.optionalAttrs (postgres != null) { postgres = postgres.publicApi or null; })
      // (pkgs.lib.optionalAttrs (nginx != null) { nginx = nginx.publicApi or null; })
      // (pkgs.lib.optionalAttrs (minio != null) { minio = minio.publicApi or null; });
  _ = serviceApi.validateServiceApis effectiveServiceApis;

  serviceEnv = serviceApi.mkServiceHookEnvFromContract effectiveServiceApis;

  supervisorEnv =
    if supervisor == null then
      { }
    else
      {
        SUPERVISOR_START = toString supervisor.start;
        SUPERVISOR_STOP = toString supervisor.stop;
        SUPERVISOR_START_DAEMON = toString supervisor.startDaemon;
        SUPERVISOR_STATUS = toString supervisor.status;
        SUPERVISOR_IS_RUNNING = toString supervisor.isRunning;
        SUPERVISOR_LOGS = toString supervisor.logs;
        SUPERVISOR_RESTART = toString supervisor.restart;
      };

  ephemeralEnv =
    if ephemeral == null then
      { }
    else
      {
        EPHEMERAL_IS_ACTIVE = toString ephemeral.isEphemeral;
        EPHEMERAL_PATHS = toString ephemeral.getEphemeralPaths;
      };
in
{
  env = {
    SLOT_INFO = toString slots.getSlotInfo;
    REQUIRE_SLOT_ENV = toString slots.requireSlotEnv;
  }
  // serviceEnv
  // supervisorEnv
  // ephemeralEnv;
}
