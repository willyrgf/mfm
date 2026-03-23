{
  apps = import ./apps.nix;
  core = import ./core.nix;
  runtime = import ./runtime.nix;
  serviceSets = import ./service-sets.nix;
  tasks = import ./tasks.nix;
  workflows = import ./workflows.nix;
  operations = import ./operations.nix;

  services = {
    postgres = import ./services/postgres.nix;
    nginx = import ./services/nginx.nix;
    minio = import ./services/minio.nix;
    reth = import ./services/reth.nix;
    helios = import ./services/helios.nix;
  };

  profiles = {
    webapp = import ./profiles/webapp.nix;
  };
}
