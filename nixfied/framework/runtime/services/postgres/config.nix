# PostgreSQL environment-specific configurations
{
  pkgs,
  project,
}:

let
  serviceConfig = import ../../../core/service-config.nix {
    lib = pkgs.lib;
    inherit pkgs;
  };
  cfg = serviceConfig.getProjectServiceConfig {
    inherit project;
    name = "postgres";
  };
  extensions = cfg.extensions or [ ];
  extraConfig = cfg.extraConfig or "";
  userEnvConfigs = cfg.envConfigs or { };

  # Base configuration shared across all environments
  baseConf = ''
    listen_addresses = 'localhost'
    # Lifecycle hooks rewrite this to the slot-specific runtime port.
    # Keep a static valid fallback so fresh initdb config always parses.
    port = 5432
    unix_socket_directories = '/tmp'
    log_destination = 'stderr'
    logging_collector = off
  '';

  # Environment-specific defaults
  devDefaults = ''
    max_connections = 50
    shared_buffers = 128MB
    fsync = off
    synchronous_commit = off
    full_page_writes = off
    wal_level = minimal
    max_wal_senders = 0
  '';

  prodDefaults = ''
    max_connections = 100
    shared_buffers = 256MB
    fsync = on
    synchronous_commit = on
    wal_level = replica
    max_wal_senders = 3
    archive_mode = on
  '';

  testDefaults = ''
    max_connections = 50
    shared_buffers = 128MB
    fsync = off
    synchronous_commit = off
    full_page_writes = off
    wal_level = minimal
    max_wal_senders = 0
    autovacuum = off
  '';

  mkEnvConf =
    envName: defaults:
    let
      userOverrides = userEnvConfigs.${envName} or { };
      userConfStr = userOverrides.extraConfig or "";
    in
    ''
      ${baseConf}
      ${defaults}
      ${extraConfig}
      ${userConfStr}
    '';

  devConf = mkEnvConf "dev" devDefaults;
  prodConf = mkEnvConf "prod" prodDefaults;
  testConf = mkEnvConf "test" testDefaults;

  mkEnvConfFile = envName: conf: pkgs.writeText "postgresql-${envName}.conf" conf;

  pgHbaConf = ''
    # TYPE  DATABASE        USER  ADDRESS       METHOD
    local   all             all                 trust
    host    all             all   127.0.0.1/32  trust
    host    all             all   ::1/128       trust
  '';
in
{
  inherit
    baseConf
    extensions
    devConf
    prodConf
    testConf
    pgHbaConf
    ;
  defaultSource = cfg.defaultSource or "";
  probePlans = cfg.resolved.probePlans or cfg.resolved.operationProbes or { };
  resolvedEndpoints = cfg.resolved.endpoints or { };
  devConfFile = mkEnvConfFile "dev" devConf;
  prodConfFile = mkEnvConfFile "prod" prodConf;
  testConfFile = mkEnvConfFile "test" testConf;
  pgHbaConfFile = pkgs.writeText "pg_hba.conf" pgHbaConf;
}
