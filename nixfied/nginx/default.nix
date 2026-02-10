# Nginx module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  templates = import ./templates.nix { inherit pkgs; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      templates
      ;
  };
  siteMgmt = import ./site-management.nix {
    inherit
      pkgs
      project
      slots
      templates
      lifecycle
      ;
  };
  ssl = import ./ssl.nix {
    inherit
      pkgs
      project
      slots
      lifecycle
      ;
  };
in
{
  # Lifecycle (backward compat)
  inherit (lifecycle)
    nginx
    init
    start
    stop
    reload
    generateSelfSignedCert
    listInstances
    ;

  # Site management (backward compat + new)
  inherit (siteMgmt)
    writeProxySite
    writeStaticSite
    addSite
    removeSite
    enableSite
    disableSite
    listSites
    ;

  # SSL
  inherit (ssl) obtainCert renewCerts certStatus;
}
