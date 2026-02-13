# Module hook registry (optional)
{
  pkgs,
  project,
  slots,
  postgres ? null,
  nginx ? null,
  minio ? null,
  reth ? null,
  helios ? null,
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
      serviceApi.mkServiceApisFromModules {
        inherit
          postgres
          nginx
          minio
          reth
          helios
          ;
      };
  _ = serviceApi.validateServiceApis effectiveServiceApis;

  serviceEnv = serviceApi.mkServiceHookEnvFromContract effectiveServiceApis;
  mkOptionalEnv = cfg: attrs: if cfg == null then { } else attrs;

  supervisorEnv = mkOptionalEnv supervisor {
    SUPERVISOR_START = toString supervisor.start;
    SUPERVISOR_STOP = toString supervisor.stop;
    SUPERVISOR_START_DAEMON = toString supervisor.startDaemon;
    SUPERVISOR_STATUS = toString supervisor.status;
    SUPERVISOR_HEALTH = toString supervisor.health;
    SUPERVISOR_IS_RUNNING = toString supervisor.isRunning;
    SUPERVISOR_LOGS = toString supervisor.logs;
    SUPERVISOR_RESTART = toString supervisor.restart;
  };

  ephemeralEnv = mkOptionalEnv ephemeral {
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
