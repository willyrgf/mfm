{ pkgs }:
let
  lib = pkgs.lib;
  registryLocksShell = import ./events-locks.nix { inherit pkgs; };
  registrySnapshotShell = import ./events-snapshot.nix { inherit pkgs; };
  registryLockMetaSuffix = ".meta";
  registryLocksDirName = "locks";
  registrySeqFileName = ".seq";
  registrySeqLockFileName = "seq.lock";
  registryEventsFileName = "events.ndjson";
  registryEventsIndexFileName = "events.index.tsv";
  registryEventsLockFileName = "events.lock";
  registryEventKind = "runtime-event";
  registryEventVersion = 1;
  registryTimestampFormat = "%Y-%m-%dT%H:%M:%SZ";
  registrySnapshotTemplate = "nixfied-events-snapshot.XXXXXX";
  registryDefaultLockTimeoutSeconds = 30;
  registryAppendShell = import ./events-append.nix { inherit pkgs; };
in
{
  mkShellLib =
    { }:
    ''
      REGISTRY_LOCK_META_SUFFIX=${lib.escapeShellArg registryLockMetaSuffix}
      REGISTRY_LOCKS_DIR_NAME=${lib.escapeShellArg registryLocksDirName}
      REGISTRY_SEQ_FILE_NAME=${lib.escapeShellArg registrySeqFileName}
      REGISTRY_SEQ_LOCK_FILE_NAME=${lib.escapeShellArg registrySeqLockFileName}
      REGISTRY_EVENTS_FILE_NAME=${lib.escapeShellArg registryEventsFileName}
      REGISTRY_EVENTS_INDEX_FILE_NAME=${lib.escapeShellArg registryEventsIndexFileName}
      REGISTRY_EVENTS_LOCK_FILE_NAME=${lib.escapeShellArg registryEventsLockFileName}
      REGISTRY_EVENT_KIND=${lib.escapeShellArg registryEventKind}
      REGISTRY_EVENT_VERSION=${lib.escapeShellArg (toString registryEventVersion)}
      REGISTRY_TIMESTAMP_FORMAT=${lib.escapeShellArg registryTimestampFormat}
      REGISTRY_SNAPSHOT_TEMPLATE=${lib.escapeShellArg registrySnapshotTemplate}
      REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS=${lib.escapeShellArg (toString registryDefaultLockTimeoutSeconds)}
      ${registryLocksShell}
      ${registrySnapshotShell}
      ${registryAppendShell}
    '';
}
