{ lib }:
let
  packagePath =
    {
      discardContext ? false,
      pkg,
    }:
    if pkg == null then
      null
    else
      let
        path = builtins.toString pkg;
      in
      if discardContext then builtins.unsafeDiscardStringContext path else path;

  dropNulls =
    attrs:
    builtins.removeAttrs attrs (
      builtins.filter (name: attrs.${name} == null) (builtins.attrNames attrs)
    );

  sortUnique = values: builtins.sort builtins.lessThan (lib.unique values);

  normalizeSource =
    discardContext: source:
    dropNulls {
      package =
        if (source ? package) && source.package != null then
          packagePath {
            inherit discardContext;
            pkg = source.package;
          }
        else
          null;
      clientPackage =
        if (source ? clientPackage) && source.clientPackage != null then
          packagePath {
            inherit discardContext;
            pkg = source.clientPackage;
          }
        else
          null;
    };

  normalizeSources =
    discardContext: keys: sources:
    builtins.listToAttrs (
      map (key: {
        name = key;
        value = normalizeSource discardContext (sources.${key} or { });
      }) keys
    );

  resolveSelectedSource =
    serviceName: keys: defaultSource:
    if defaultSource == "" then
      ""
    else if builtins.elem defaultSource keys then
      defaultSource
    else
      throw "service '${serviceName}' defaultSource '${defaultSource}' is not defined in sources";

  sourceKeys = cfg: sortUnique ((cfg.sourceKeys or [ ]) ++ builtins.attrNames (cfg.sources or { }));

  sourceValue =
    serviceName: cfg:
    let
      keys = sourceKeys cfg;
      sources = cfg.sources or { };
      selectedSource = resolveSelectedSource serviceName keys (cfg.defaultSource or "");
    in
    if selectedSource == "" then
      {
        inherit selectedSource;
        value = { };
      }
    else
      {
        inherit selectedSource;
        value = sources.${selectedSource} or { };
      };

  normalizePostgres =
    discardContext: cfg:
    let
      keys = sourceKeys cfg;
      normalizedSources = normalizeSources discardContext keys (cfg.sources or { });
      selected = sourceValue "postgres" cfg;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
    in
    cfg
    // {
      sources = normalizedSources;
      sourceKeys = keys;
      package = package;
      resolved = {
        sources = normalizedSources;
        selectedSource = selected.selectedSource;
        endpoints = {
          primary = {
            protocol = "postgres";
            portKey = cfg.portKey;
          };
        };
        probes = {
          health = "pg_isready";
          ready = "sql";
        };
        operationProbes = {
          health = {
            count = 1;
            steps = [
              {
                kind = "postgres-pg-isready";
                endpoint = "primary";
                serviceLabel = "postgres";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
                host = "127.0.0.1";
                failureSuffix = "";
              }
            ];
          };
          ready = {
            count = 1;
            steps = [
              {
                kind = "postgres-pg-isready";
                endpoint = "primary";
                serviceLabel = "postgres";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
                host = "127.0.0.1";
                failureSuffix = " (pg_isready failed)";
              }
              {
                kind = "postgres-query";
                endpoint = "primary";
                serviceLabel = "postgres";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
                host = "127.0.0.1";
                database = cfg.database;
                query = "select 1;";
                failureSuffix = " (query failed)";
              }
            ];
          };
        };
        paths = {
          dataDirName = cfg.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          database = cfg.database;
          testDatabase = cfg.testDatabase;
        };
        migrations = {
          dir = cfg.migrations.dir;
          command = cfg.migrations.command;
          sourceDatabase = cfg.migrations.sourceDatabase;
        };
      };
    };

  normalizeNginx =
    discardContext: cfg:
    let
      keys = sourceKeys cfg;
      normalizedSources = normalizeSources discardContext keys (cfg.sources or { });
      selected = sourceValue "nginx" cfg;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
    in
    cfg
    // {
      sources = normalizedSources;
      sourceKeys = keys;
      package = package;
      resolved = {
        sources = normalizedSources;
        selectedSource = selected.selectedSource;
        endpoints = {
          http = {
            protocol = "http";
            portKey = cfg.portKeyHttp;
          };
          https = {
            protocol = "https";
            portKey = cfg.portKeyHttps;
          };
        };
        probes = {
          health = "tcp";
          ready = "tcp";
        };
        operationProbes = {
          health = {
            count = 2;
            steps = [
              {
                kind = "tcp";
                endpoint = "http";
                serviceLabel = "nginx";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
              {
                kind = "tcp";
                endpoint = "https";
                serviceLabel = "nginx";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
            ];
          };
          ready = {
            count = 2;
            steps = [
              {
                kind = "tcp";
                endpoint = "http";
                serviceLabel = "nginx";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
              {
                kind = "tcp";
                endpoint = "https";
                serviceLabel = "nginx";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
            ];
          };
        };
        paths = {
          dataDirName = cfg.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
        };
      };
    };

  normalizeMinio =
    discardContext: cfg:
    let
      keys = sourceKeys cfg;
      normalizedSources = normalizeSources discardContext keys (cfg.sources or { });
      selected = sourceValue "minio" cfg;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
      clientPackage =
        if (selected.value ? clientPackage) && selected.value.clientPackage != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.clientPackage;
          }
        else
          null;
    in
    cfg
    // {
      sources = normalizedSources;
      sourceKeys = keys;
      package = package;
      clientPackage = clientPackage;
      resolved = {
        sources = normalizedSources;
        selectedSource = selected.selectedSource;
        endpoints = {
          api = {
            protocol = "http";
            portKey = cfg.portKeyApi;
          };
          console = {
            protocol = "http";
            portKey = cfg.portKeyConsole;
          };
        };
        probes = {
          health = "/minio/health/live";
          ready = "/minio/health/ready";
        };
        operationProbes = {
          health = {
            count = 2;
            steps = [
              {
                kind = "tcp";
                endpoint = "api";
                serviceLabel = "minio";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
              {
                kind = "tcp";
                endpoint = "console";
                serviceLabel = "minio";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
            ];
          };
          ready = {
            count = 2;
            steps = [
              {
                kind = "tcp";
                endpoint = "api";
                serviceLabel = "minio";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
              {
                kind = "tcp";
                endpoint = "console";
                serviceLabel = "minio";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
            ];
          };
        };
        paths = {
          dataDirName = cfg.dataDirName;
        };
        runtime = dropNulls {
          inherit package clientPackage;
          browser = cfg.browser;
        };
      };
    };

  normalizeReth =
    discardContext: cfg:
    let
      keys = sourceKeys cfg;
      normalizedSources = normalizeSources discardContext keys (cfg.sources or { });
      selected = sourceValue "reth" cfg;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
    in
    cfg
    // {
      sources = normalizedSources;
      sourceKeys = keys;
      package = package;
      resolved = {
        sources = normalizedSources;
        selectedSource = selected.selectedSource;
        endpoints = {
          http = {
            protocol = "http";
            portKey = cfg.portKeyHttp;
          };
          ws = {
            protocol = "ws";
            portKey = cfg.portKeyWs;
          };
          auth = {
            protocol = "http";
            portKey = cfg.portKeyAuth;
          };
        };
        probes = {
          health = "jsonrpc:web3_clientVersion";
          ready = "jsonrpc:web3_clientVersion";
        };
        operationProbes = {
          health = {
            count = 3;
            steps = [
              {
                kind = "jsonrpc";
                endpoint = "http";
                serviceLabel = "reth";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
                method = "web3_clientVersion";
              }
              {
                kind = "tcp";
                endpoint = "ws";
                serviceLabel = "reth";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
              {
                kind = "tcp";
                endpoint = "auth";
                serviceLabel = "reth";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
              }
            ];
          };
          ready = {
            count = 3;
            steps = [
              {
                kind = "jsonrpc";
                endpoint = "http";
                serviceLabel = "reth";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
                method = "eth_chainId";
              }
              {
                kind = "tcp";
                endpoint = "ws";
                serviceLabel = "reth";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
              {
                kind = "tcp";
                endpoint = "auth";
                serviceLabel = "reth";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
              }
            ];
          };
        };
        paths = {
          dataDirName = cfg.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          network = cfg.network;
          devMode = cfg.devMode;
        };
      };
    };

  normalizeHelios =
    discardContext: cfg:
    let
      keys = sourceKeys cfg;
      normalizedSources = normalizeSources discardContext keys (cfg.sources or { });
      selected = sourceValue "helios" cfg;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
    in
    cfg
    // {
      sources = normalizedSources;
      sourceKeys = keys;
      package = package;
      resolved = {
        sources = normalizedSources;
        selectedSource = selected.selectedSource;
        endpoints = {
          rpc = {
            protocol = "http";
            portKey = cfg.portKeyRpc;
          };
          execution = {
            protocol = "http";
            portKey = cfg.executionRpcPortKey;
          };
        };
        probes = {
          health = "jsonrpc:eth_chainId";
          ready = "jsonrpc:eth_blockNumber";
        };
        operationProbes = {
          health = {
            count = 2;
            steps = [
              {
                kind = "jsonrpc";
                endpoint = "rpc";
                serviceLabel = "helios";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
                method = "eth_chainId";
              }
              {
                kind = "jsonrpc";
                endpoint = "execution";
                serviceLabel = "helios execution";
                phaseLabel = "health";
                successLabel = "healthy";
                failureLabel = "unhealthy";
                method = "web3_clientVersion";
              }
            ];
          };
          ready = {
            count = 2;
            steps = [
              {
                kind = "helios-ready";
                endpoint = "rpc";
                executionEndpoint = "execution";
                serviceLabel = "helios";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
                sourceKinds = cfg.sourceKinds or { };
                readinessProfile = cfg.readiness.profile or "fast";
                requireNotSyncing =
                  (cfg.readiness.requireNotSyncing or false) || (cfg.readiness.profile or "fast") == "strict";
                disallowSourceKinds = lib.unique (
                  (cfg.readiness.disallowSourceKinds or [ ])
                  ++ lib.optionals ((cfg.readiness.profile or "fast") == "strict") [
                    "shim"
                    "unknown"
                  ]
                );
              }
              {
                kind = "jsonrpc";
                endpoint = "execution";
                serviceLabel = "helios execution";
                phaseLabel = "readiness";
                successLabel = "ready";
                failureLabel = "not ready";
                method = "eth_chainId";
              }
            ];
          };
        };
        paths = {
          dataDirName = cfg.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          network = cfg.network;
          executionRpcUrl = cfg.executionRpcUrl;
          consensusRpcUrl = cfg.consensusRpcUrl;
          checkpoint = cfg.checkpoint;
        };
      };
    };

  normalizeByName =
    discardContext: name: cfg:
    if name == "postgres" then
      normalizePostgres discardContext cfg
    else if name == "nginx" then
      normalizeNginx discardContext cfg
    else if name == "minio" then
      normalizeMinio discardContext cfg
    else if name == "reth" then
      normalizeReth discardContext cfg
    else if name == "helios" then
      normalizeHelios discardContext cfg
    else
      cfg;

  getProjectServiceEntry =
    {
      project,
      name,
    }:
    let
      services = project.services or { };
      serviceId = "service.${name}";
    in
    if builtins.hasAttr name services then
      services.${name}
    else if builtins.hasAttr serviceId services then
      services.${serviceId}
    else
      null;
in
{
  normalizeServiceConfig =
    {
      discardContext ? false,
      name,
      config,
    }:
    normalizeByName discardContext name config;

  inherit getProjectServiceEntry;

  getProjectServiceConfig =
    {
      project,
      name,
    }:
    let
      entry = getProjectServiceEntry {
        inherit
          project
          name
          ;
      };
      rawConfig =
        if entry == null then
          { }
        else if builtins.hasAttr "config" entry then
          entry.config
        else
          builtins.removeAttrs entry [
            "enable"
            "id"
            "name"
          ];
    in
    normalizeByName false name rawConfig;

  isProjectServiceEnabled =
    {
      project,
      name,
    }:
    let
      entry = getProjectServiceEntry {
        inherit
          project
          name
          ;
      };
    in
    if entry == null then false else entry.enable or false;
}
