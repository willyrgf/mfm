# PostgreSQL module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  config = import ./config.nix { inherit pkgs project; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };
  backupMod = import ./backup.nix { inherit pkgs project slots; };
  migration = import ./migration.nix { inherit pkgs project slots; };
  migrationSafety = import ./migration-safety.nix { inherit pkgs project slots; };
  rollback = import ./rollback.nix { inherit pkgs project slots; };
  portMgmt = import ./port-management.nix { inherit pkgs; };
in
{
  # Lifecycle (backward compat)
  inherit (lifecycle)
    postgres
    init
    start
    stop
    setupDb
    fullStart
    fullStartTest
    listInstances
    ;

  # Backup
  inherit (backupMod)
    archiveWal
    setupArchiving
    backup
    restore
    listBackups
    verifyBackup
    cleanupBackups
    ;

  # Migration
  inherit (migration) testMigrations;

  # Migration safety
  inherit (migrationSafety)
    getMigrationHash
    ensureMigrationTested
    markMigrationTested
    detectDrift
    ;

  # Rollback
  inherit (rollback) findBackupForCommit testRollback;

  # Port management
  inherit (portMgmt)
    checkPort
    getPortPids
    getPortInfo
    killPort
    assertPortsFree
    ;

  # Config (for reference)
  inherit config;
}
