{
  helios = {
    apps = [
      { opName = "check-config"; }
      { opName = "full-start"; }
      { opName = "full-start-test"; }
      { opName = "health"; }
      { opName = "init"; }
      { opName = "ready"; }
      { opName = "restart"; }
      { opName = "start"; }
      { opName = "status"; }
      { opName = "stop"; }
    ];
  };

  minio = {
    apps = [
      { opName = "bucket-create"; }
      { opName = "bucket-delete"; }
      { opName = "bucket-ensure"; }
      { opName = "bucket-list"; }
      { opName = "check-config"; }
      { opName = "export-s3-env"; }
      { opName = "full-start"; }
      { opName = "full-start-test"; }
      { opName = "health"; }
      { opName = "init"; }
      { opName = "policy-apply"; }
      { opName = "ready"; }
      { opName = "restart"; }
      { opName = "start"; }
      { opName = "status"; }
      { opName = "stop"; }
    ];
  };

  nginx = {
    apps = [
      { opName = "cert-obtain"; }
      { opName = "cert-renew"; }
      { opName = "cert-status"; }
      { opName = "check-config"; }
      { opName = "health"; }
      { opName = "init"; }
      { opName = "list-instances"; }
      { opName = "ready"; }
      { opName = "reload"; }
      { opName = "restart"; }
      { opName = "site-add"; }
      { opName = "site-disable"; }
      { opName = "site-enable"; }
      { opName = "site-list"; }
      { opName = "site-remove"; }
      { opName = "start"; }
      { opName = "status"; }
      { opName = "stop"; }
    ];
  };

  postgres = {
    apps = [
      { opName = "backup"; }
      { opName = "check-config"; }
      { opName = "check-port"; }
      { opName = "cleanup-backups"; }
      { opName = "full-start"; }
      { opName = "full-start-test"; }
      { opName = "health"; }
      { opName = "init"; }
      { opName = "kill-port"; }
      { opName = "list-backups"; }
      { opName = "list-instances"; }
      { opName = "ready"; }
      { opName = "ready-test"; }
      { opName = "restart"; }
      { opName = "restore"; }
      { opName = "setup-db"; }
      { opName = "shell"; }
      { opName = "start"; }
      { opName = "status"; }
      { opName = "stop"; }
      { opName = "test-migrations"; }
      { opName = "verify-backup"; }
    ];
  };

  reth = {
    apps = [
      { opName = "check-config"; }
      { opName = "full-start"; }
      { opName = "full-start-test"; }
      { opName = "health"; }
      { opName = "init"; }
      { opName = "ready"; }
      { opName = "restart"; }
      { opName = "start"; }
      { opName = "status"; }
      { opName = "stop"; }
    ];
  };
}
