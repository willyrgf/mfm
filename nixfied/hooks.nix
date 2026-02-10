# Module hook registry (optional)
{
  pkgs,
  project,
  slots,
  postgres ? null,
  nginx ? null,
  supervisor ? null,
  ephemeral ? null,
}:

let
  postgresEnv =
    if postgres == null then
      { }
    else
      {
        POSTGRES_INIT = toString postgres.init;
        POSTGRES_START = toString postgres.start;
        POSTGRES_STOP = toString postgres.stop;
        POSTGRES_SETUP_DB = toString postgres.setupDb;
        POSTGRES_FULL_START = toString postgres.fullStart;
        POSTGRES_FULL_START_TEST = toString postgres.fullStartTest;
        POSTGRES_BACKUP = toString postgres.backup;
        POSTGRES_RESTORE = toString postgres.restore;
        POSTGRES_LIST_BACKUPS = toString postgres.listBackups;
        POSTGRES_TEST_MIGRATIONS = toString postgres.testMigrations;
        POSTGRES_ENSURE_MIGRATION_TESTED = toString postgres.ensureMigrationTested;
        POSTGRES_CHECK_PORT = toString postgres.checkPort;
        POSTGRES_KILL_PORT = toString postgres.killPort;
        POSTGRES_LIST_INSTANCES = toString postgres.listInstances;
      };

  nginxEnv =
    if nginx == null then
      { }
    else
      {
        NGINX_INIT = toString nginx.init;
        NGINX_START = toString nginx.start;
        NGINX_STOP = toString nginx.stop;
        NGINX_RELOAD = toString nginx.reload;
        NGINX_SITE_PROXY = toString nginx.writeProxySite;
        NGINX_SITE_STATIC = toString nginx.writeStaticSite;
        NGINX_SITE_ADD = toString nginx.addSite;
        NGINX_SITE_REMOVE = toString nginx.removeSite;
        NGINX_SITE_LIST = toString nginx.listSites;
        NGINX_SITE_ENABLE = toString nginx.enableSite;
        NGINX_SITE_DISABLE = toString nginx.disableSite;
        NGINX_CERT_OBTAIN = toString nginx.obtainCert;
        NGINX_CERT_RENEW = toString nginx.renewCerts;
        NGINX_CERT_STATUS = toString nginx.certStatus;
      };

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
  // postgresEnv
  // nginxEnv
  // supervisorEnv
  // ephemeralEnv;
}
