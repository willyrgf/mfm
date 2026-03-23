# Nginx module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  summary = import ../../helpers/summary.nix { inherit pkgs project; };
  helpers = import ../../helpers/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  serviceModule = import ../../helpers/service-module.nix {
    inherit
      pkgs
      project
      slots
      ;
  };
  config = import ./config.nix { inherit pkgs project; };
  portVarHttp = slots.portVarName (config.portKeyHttp or "http");
  portVarHttps = slots.portVarName (config.portKeyHttps or "https");
  dataDirName = config.dataDirName or "nginx";
  nginxDirExpr = slots.getServiceDir dataDirName;

  templates = import ./templates.nix {
    inherit pkgs;
    package = config.package or pkgs.nginx;
  };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      templates
      loggingPrelude
      ;
  };
  siteMgmt = import ./site-management.nix {
    inherit
      pkgs
      project
      slots
      config
      templates
      lifecycle
      loggingPrelude
      ;
  };
  ssl = import ./ssl.nix {
    inherit
      pkgs
      project
      slots
      config
      lifecycle
      loggingPrelude
      ;
  };
in
serviceModule.mkServiceModule {
  service = "nginx";
  summaryName = "nginx";
  summary = "Nginx service management API";
  details = "Public service contract for managing nginx across dev/prod/test/ci.";
  artifacts = {
    httpPortVar = portVarHttp;
    httpsPortVar = portVarHttps;
    serviceDir = nginxDirExpr;
    dataDir = nginxDirExpr;
    logFile = "${nginxDirExpr}/logs/error.log";
    pidFile = "${nginxDirExpr}/run/nginx.pid";
  };
  inherit config;
  operations = {
    init = {
      script = lifecycle.init;
      summary = "Initialize nginx directories and config";
      details = "Creates nginx runtime directories and base configuration.";
    };
    start = {
      script = lifecycle.start;
      summary = "Start nginx server";
      details = "Starts nginx for the current slot and environment.";
    };
    stop = {
      script = lifecycle.stop;
      summary = "Stop nginx server";
      details = "Stops nginx for the current slot and environment.";
    };
    restart = {
      script = lifecycle.restart;
      summary = "Restart nginx server";
      details = "Stops then starts nginx for the current slot and environment.";
    };
    status = {
      script = lifecycle.status;
      summary = "Show nginx status";
      details = "Prints nginx status for the current slot and environment.";
    };
    health = {
      script = lifecycle.health;
      summary = "Run nginx health check";
      details = "Checks that nginx responds on the configured HTTP port.";
    };
    check-config = {
      script = lifecycle.checkConfig;
      summary = "Validate nginx configuration";
      details = "Runs nginx config validation for the current slot and environment.";
    };
    reload = {
      script = lifecycle.reload;
      summary = "Reload nginx configuration";
      details = "Tests and reloads nginx configuration.";
    };
    ready = {
      script = lifecycle.ready;
      hook = "READY";
      summary = "Wait for nginx readiness";
      details = "Checks that nginx serves HTTP requests on the configured port.";
    };
    list-instances = {
      script = lifecycle.listInstances;
      hook = "LIST_INSTANCES";
      summary = "List nginx instances";
      details = "Lists nginx instances managed by Nixfied.";
    };
    site-proxy = {
      script = siteMgmt.writeProxySite;
      hook = "SITE_PROXY";
      summary = "Write proxy site configuration";
      details = "Writes a proxy site config and enables it.";
      exposeApp = false;
      usage = [ "nix run .#svc::nginx::site-proxy -- <domain> <upstream-host> <upstream-port>" ];
    };
    site-static = {
      script = siteMgmt.writeStaticSite;
      hook = "SITE_STATIC";
      summary = "Write static site configuration";
      details = "Writes a static site config and enables it.";
      exposeApp = false;
      usage = [ "nix run .#svc::nginx::site-static -- <domain> <site-root>" ];
    };
    site-add = {
      script = siteMgmt.addSite;
      hook = "SITE_ADD";
      summary = "Add proxy nginx site";
      details = "Adds a proxy site and enables it.";
      usage = [ "nix run .#svc::nginx::site-add -- <domain> <upstream-host> <upstream-port>" ];
    };
    site-remove = {
      script = siteMgmt.removeSite;
      hook = "SITE_REMOVE";
      summary = "Remove nginx site";
      details = "Removes nginx site configuration.";
      usage = [ "nix run .#svc::nginx::site-remove -- <domain>" ];
    };
    site-list = {
      script = siteMgmt.listSites;
      hook = "SITE_LIST";
      summary = "List nginx sites";
      details = "Lists configured nginx sites.";
    };
    site-enable = {
      script = siteMgmt.enableSite;
      hook = "SITE_ENABLE";
      summary = "Enable nginx site";
      details = "Enables an existing nginx site.";
      usage = [ "nix run .#svc::nginx::site-enable -- <domain>" ];
    };
    site-disable = {
      script = siteMgmt.disableSite;
      hook = "SITE_DISABLE";
      summary = "Disable nginx site";
      details = "Disables an existing nginx site.";
      usage = [ "nix run .#svc::nginx::site-disable -- <domain>" ];
    };
    cert-obtain = {
      script = ssl.obtainCert;
      hook = "CERT_OBTAIN";
      summary = "Obtain SSL certificate";
      details = "Obtains a Let's Encrypt certificate for a domain.";
      usage = [ "nix run .#svc::nginx::cert-obtain -- <domain> <email> [--staging]" ];
    };
    cert-renew = {
      script = ssl.renewCerts;
      hook = "CERT_RENEW";
      summary = "Renew SSL certificates";
      details = "Renews certificates for configured sites.";
    };
    cert-status = {
      script = ssl.certStatus;
      hook = "CERT_STATUS";
      summary = "Show SSL certificate status";
      details = "Prints certificate status for configured domains.";
    };
  };
  exported = {
    inherit (lifecycle)
      nginx
      init
      start
      stop
      restart
      status
      health
      ready
      checkConfig
      reload
      generateSelfSignedCert
      listInstances
      ;

    inherit (siteMgmt)
      writeProxySite
      writeStaticSite
      addSite
      removeSite
      enableSite
      disableSite
      listSites
      ;

    inherit (ssl) obtainCert renewCerts certStatus;
  };
}
