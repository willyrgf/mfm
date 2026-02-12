# PostgreSQL environment-specific configurations
{
  pkgs,
  project,
}:

let
  cfg = project.modules.postgres or { };
  extensions = cfg.extensions or [ ];
  extraConfig = cfg.extraConfig or "";
  userEnvConfigs = cfg.envConfigs or { };

  # Base configuration shared across all environments
  baseConf = ''
    listen_addresses = 'localhost'
    # The lifecycle start hook sets the slot-specific port via pg_ctl -o "-p ...".
    # Keep postgresql.conf static/valid so fresh initdb directories start cleanly.
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
in
{
  devConf = mkEnvConf "dev" devDefaults;
  prodConf = mkEnvConf "prod" prodDefaults;
  testConf = mkEnvConf "test" testDefaults;
  inherit baseConf extensions;
}
