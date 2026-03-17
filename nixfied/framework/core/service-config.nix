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

  defaultWait = {
    enabled = false;
    timeoutSeconds = 300;
    intervalSeconds = 1;
    timeoutEnvVar = null;
    intervalEnvVar = null;
  };

  normalizeWait =
    wait:
    defaultWait
    // dropNulls {
      enabled = if wait ? enabled then wait.enabled else null;
      timeoutSeconds = if wait ? timeoutSeconds then wait.timeoutSeconds else null;
      intervalSeconds = if wait ? intervalSeconds then wait.intervalSeconds else null;
      timeoutEnvVar = if wait ? timeoutEnvVar then wait.timeoutEnvVar else null;
      intervalEnvVar = if wait ? intervalEnvVar then wait.intervalEnvVar else null;
    };

  mergeWait =
    base: override: if override == null then normalizeWait base else normalizeWait (base // override);

  modePhaseLabel = mode: if mode == "health" then "health" else "readiness";
  modeSuccessLabel = mode: if mode == "health" then "healthy" else "ready";
  modeFailureLabel = mode: if mode == "health" then "unhealthy" else "not ready";

  requireValue =
    {
      serviceName,
      mode,
      kind,
      field,
      value,
    }:
    if value == null || value == "" then
      throw "service '${serviceName}' probe '${mode}' kind '${kind}' requires field '${field}'"
    else
      value;

  stepLabelDefault =
    serviceName: step:
    if step.label or null != null then
      step.label
    else if step.endpoint or null == "execution" then
      "${serviceName} execution"
    else
      serviceName;

  normalizeOverrideStep =
    serviceName: mode: step:
    let
      kind = step.kind;
      stepBase = {
        inherit kind;
        serviceLabel = stepLabelDefault serviceName step;
        phaseLabel = modePhaseLabel mode;
        successLabel = modeSuccessLabel mode;
        failureLabel = modeFailureLabel mode;
      };
    in
    if kind == "tcp" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
      }
    else if kind == "http" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
        path = step.path or "/";
      }
    else if kind == "jsonrpc" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
        method = requireValue {
          inherit serviceName mode kind;
          field = "method";
          value = step.method or "";
        };
      }
    else if kind == "postgres-pg-isready" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
        host = step.host or "127.0.0.1";
        failureSuffix = "";
      }
    else if kind == "postgres-query" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
        host = step.host or "127.0.0.1";
        database = requireValue {
          inherit serviceName mode kind;
          field = "database";
          value = step.database or null;
        };
        query = requireValue {
          inherit serviceName mode kind;
          field = "query";
          value = step.query or "";
        };
        failureSuffix = " (query failed)";
      }
    else if kind == "helios-ready" then
      stepBase
      // {
        endpoint = requireValue {
          inherit serviceName mode kind;
          field = "endpoint";
          value = step.endpoint or null;
        };
        executionEndpoint = requireValue {
          inherit serviceName mode kind;
          field = "executionEndpoint";
          value = step.executionEndpoint or null;
        };
        sourceKinds = step.sourceKinds or { };
        readinessProfile = step.readinessProfile or "fast";
        requireNotSyncing = step.requireNotSyncing or false;
        disallowSourceKinds = step.disallowSourceKinds or [ ];
      }
    else if kind == "exec" then
      stepBase
      // {
        command = requireValue {
          inherit serviceName mode kind;
          field = "command";
          value = step.command or "";
        };
      }
    else
      throw "service '${serviceName}' probe '${mode}' uses unsupported kind '${kind}'";

  mergeProbePlan =
    serviceName: mode: basePlan: override:
    let
      normalizedBase = basePlan // {
        wait = normalizeWait (basePlan.wait or { });
      };
    in
    if override == null then
      normalizedBase
    else
      let
        overrideSteps = map (normalizeOverrideStep serviceName mode) (override.steps or [ ]);
        strategy = override.strategy or "replace";
        mergedSteps =
          if overrideSteps == [ ] then
            normalizedBase.steps
          else if strategy == "replace" then
            overrideSteps
          else if strategy == "prepend" then
            overrideSteps ++ normalizedBase.steps
          else if strategy == "append" then
            normalizedBase.steps ++ overrideSteps
          else
            throw "service '${serviceName}' probe '${mode}' uses unsupported strategy '${strategy}'";
      in
      normalizedBase
      // {
        steps = mergedSteps;
        count = builtins.length mergedSteps;
        wait = mergeWait normalizedBase.wait (override.wait or null);
      };

  mergeProbePlans = serviceName: defaultPlans: userOverrides: {
    health = mergeProbePlan serviceName "health" defaultPlans.health (userOverrides.health or null);
    ready = mergeProbePlan serviceName "ready" defaultPlans.ready (userOverrides.ready or null);
  };

  resolveProbeSummary =
    mode: defaultName: override:
    if override == null then
      defaultName
    else
      let
        steps = override.steps or [ ];
      in
      if steps == [ ] then
        defaultName
      else if builtins.length steps == 1 then
        (builtins.elemAt steps 0).kind or "custom"
      else
        "custom";

  normalizePostgres =
    discardContext: cfg:
    let
      migrations = cfg.migrations or { };
      cfgWithDefaults = cfg // {
        database = cfg.database or "app";
        testDatabase = cfg.testDatabase or "app_test";
        portKey = cfg.portKey or "postgres";
        dataDirName = cfg.dataDirName or "postgres";
        migrations = {
          dir = migrations.dir or "migrations";
          command = migrations.command or "";
          sourceDatabase = migrations.sourceDatabase or null;
        };
      };
      keys = sourceKeys cfgWithDefaults;
      normalizedSources = normalizeSources discardContext keys (cfgWithDefaults.sources or { });
      selected = sourceValue "postgres" cfgWithDefaults;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
      defaultProbePlans = {
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
          wait = defaultWait;
        };
        ready = {
          count = 2;
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
              database = cfgWithDefaults.database;
              query = "select 1;";
              failureSuffix = " (query failed)";
            }
          ];
          wait = defaultWait;
        };
      };
      probePlans = mergeProbePlans "postgres" defaultProbePlans (cfgWithDefaults.probes or { });
    in
    cfgWithDefaults
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
            portKey = cfgWithDefaults.portKey;
          };
        };
        probes = {
          health = resolveProbeSummary "health" "pg_isready" (cfgWithDefaults.probes.health or null);
          ready = resolveProbeSummary "ready" "sql" (cfgWithDefaults.probes.ready or null);
        };
        probePlans = probePlans;
        operationProbes = probePlans;
        paths = {
          dataDirName = cfgWithDefaults.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          database = cfgWithDefaults.database;
          testDatabase = cfgWithDefaults.testDatabase;
        };
        migrations = {
          dir = cfgWithDefaults.migrations.dir;
          command = cfgWithDefaults.migrations.command;
          sourceDatabase = cfgWithDefaults.migrations.sourceDatabase;
        };
      };
    };

  normalizeNginx =
    discardContext: cfg:
    let
      cfgWithDefaults = cfg // {
        portKeyHttp = cfg.portKeyHttp or "http";
        portKeyHttps = cfg.portKeyHttps or "https";
        dataDirName = cfg.dataDirName or "nginx";
      };
      keys = sourceKeys cfgWithDefaults;
      normalizedSources = normalizeSources discardContext keys (cfgWithDefaults.sources or { });
      selected = sourceValue "nginx" cfgWithDefaults;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
      defaultProbePlans = {
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
          wait = defaultWait;
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
          wait = defaultWait;
        };
      };
      probePlans = mergeProbePlans "nginx" defaultProbePlans (cfgWithDefaults.probes or { });
    in
    cfgWithDefaults
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
            portKey = cfgWithDefaults.portKeyHttp;
          };
          https = {
            protocol = "https";
            portKey = cfgWithDefaults.portKeyHttps;
          };
        };
        probes = {
          health = resolveProbeSummary "health" "tcp" (cfgWithDefaults.probes.health or null);
          ready = resolveProbeSummary "ready" "tcp" (cfgWithDefaults.probes.ready or null);
        };
        probePlans = probePlans;
        operationProbes = probePlans;
        paths = {
          dataDirName = cfgWithDefaults.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
        };
      };
    };

  normalizeMinio =
    discardContext: cfg:
    let
      cfgWithDefaults = cfg // {
        portKeyApi = cfg.portKeyApi or "minioApi";
        portKeyConsole = cfg.portKeyConsole or "minioConsole";
        dataDirName = cfg.dataDirName or "minio";
        browser = if cfg ? browser then cfg.browser else true;
      };
      keys = sourceKeys cfgWithDefaults;
      normalizedSources = normalizeSources discardContext keys (cfgWithDefaults.sources or { });
      selected = sourceValue "minio" cfgWithDefaults;
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
      defaultProbePlans = {
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
          wait = defaultWait;
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
          wait = defaultWait;
        };
      };
      probePlans = mergeProbePlans "minio" defaultProbePlans (cfgWithDefaults.probes or { });
    in
    cfgWithDefaults
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
            portKey = cfgWithDefaults.portKeyApi;
          };
          console = {
            protocol = "http";
            portKey = cfgWithDefaults.portKeyConsole;
          };
        };
        probes = {
          health = resolveProbeSummary "health" "/minio/health/live" (cfgWithDefaults.probes.health or null);
          ready = resolveProbeSummary "ready" "/minio/health/ready" (cfgWithDefaults.probes.ready or null);
        };
        probePlans = probePlans;
        operationProbes = probePlans;
        paths = {
          dataDirName = cfgWithDefaults.dataDirName;
        };
        runtime = dropNulls {
          inherit package clientPackage;
          browser = cfgWithDefaults.browser;
        };
      };
    };

  normalizeReth =
    discardContext: cfg:
    let
      cfgWithDefaults = cfg // {
        portKeyHttp = cfg.portKeyHttp or "rethHttp";
        portKeyWs = cfg.portKeyWs or "rethWs";
        portKeyAuth = cfg.portKeyAuth or "rethAuth";
        dataDirName = cfg.dataDirName or "reth";
        network = cfg.network or "local";
        devMode = cfg.devMode or false;
      };
      keys = sourceKeys cfgWithDefaults;
      normalizedSources = normalizeSources discardContext keys (cfgWithDefaults.sources or { });
      selected = sourceValue "reth" cfgWithDefaults;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
      defaultProbePlans = {
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
          wait = defaultWait;
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
          wait = defaultWait;
        };
      };
      probePlans = mergeProbePlans "reth" defaultProbePlans (cfgWithDefaults.probes or { });
    in
    cfgWithDefaults
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
            portKey = cfgWithDefaults.portKeyHttp;
          };
          ws = {
            protocol = "ws";
            portKey = cfgWithDefaults.portKeyWs;
          };
          auth = {
            protocol = "http";
            portKey = cfgWithDefaults.portKeyAuth;
          };
        };
        probes = {
          health = resolveProbeSummary "health" "jsonrpc:web3_clientVersion" (
            cfgWithDefaults.probes.health or null
          );
          ready = resolveProbeSummary "ready" "jsonrpc:web3_clientVersion" (
            cfgWithDefaults.probes.ready or null
          );
        };
        probePlans = probePlans;
        operationProbes = probePlans;
        paths = {
          dataDirName = cfgWithDefaults.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          network = cfgWithDefaults.network;
          devMode = cfgWithDefaults.devMode;
        };
      };
    };

  normalizeHelios =
    discardContext: cfg:
    let
      readiness = cfg.readiness or { };
      cfgWithDefaults = cfg // {
        portKeyRpc = cfg.portKeyRpc or "heliosRpc";
        executionRpcPortKey = cfg.executionRpcPortKey or "rethHttp";
        dataDirName = cfg.dataDirName or "helios";
        network = cfg.network or "local";
        executionRpcUrl = cfg.executionRpcUrl or "";
        consensusRpcUrl = cfg.consensusRpcUrl or "";
        checkpoint = cfg.checkpoint or "";
        sourceKinds = cfg.sourceKinds or { };
        readiness = {
          profile = readiness.profile or "fast";
          requireNotSyncing = readiness.requireNotSyncing or false;
          disallowSourceKinds = readiness.disallowSourceKinds or [ ];
        };
      };
      keys = sourceKeys cfgWithDefaults;
      normalizedSources = normalizeSources discardContext keys (cfgWithDefaults.sources or { });
      selected = sourceValue "helios" cfgWithDefaults;
      package =
        if (selected.value ? package) && selected.value.package != null then
          packagePath {
            inherit discardContext;
            pkg = selected.value.package;
          }
        else
          null;
      defaultHeliosReadyStep = {
        kind = "helios-ready";
        endpoint = "rpc";
        executionEndpoint = "execution";
        serviceLabel = "helios";
        phaseLabel = "readiness";
        successLabel = "ready";
        failureLabel = "not ready";
        sourceKinds = cfgWithDefaults.sourceKinds;
        readinessProfile = cfgWithDefaults.readiness.profile;
        requireNotSyncing =
          cfgWithDefaults.readiness.requireNotSyncing || cfgWithDefaults.readiness.profile == "strict";
        allowLocalHealthFallback = cfgWithDefaults.network == "local";
        disallowSourceKinds = lib.unique (
          cfgWithDefaults.readiness.disallowSourceKinds
          ++ lib.optionals (cfgWithDefaults.readiness.profile == "strict") [
            "shim"
            "unknown"
          ]
        );
      };
      defaultProbePlans = {
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
          wait = defaultWait;
        };
        ready = {
          count = 2;
          steps = [
            defaultHeliosReadyStep
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
          wait = {
            enabled = true;
            timeoutSeconds = 300;
            intervalSeconds = 1;
            timeoutEnvVar = "HELIOS_READY_TIMEOUT_SECS";
            intervalEnvVar = "HELIOS_READY_INTERVAL_SECS";
          };
        };
      };
      probePlans = mergeProbePlans "helios" defaultProbePlans (cfgWithDefaults.probes or { });
    in
    cfgWithDefaults
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
            portKey = cfgWithDefaults.portKeyRpc;
          };
          execution = {
            protocol = "http";
            portKey = cfgWithDefaults.executionRpcPortKey;
          };
        };
        probes = {
          health = resolveProbeSummary "health" "jsonrpc:eth_chainId" (cfgWithDefaults.probes.health or null);
          ready = resolveProbeSummary "ready" "jsonrpc:eth_blockNumber" (
            cfgWithDefaults.probes.ready or null
          );
        };
        probePlans = probePlans;
        operationProbes = probePlans;
        paths = {
          dataDirName = cfgWithDefaults.dataDirName;
        };
        runtime = dropNulls {
          inherit package;
          network = cfgWithDefaults.network;
          executionRpcUrl = cfgWithDefaults.executionRpcUrl;
          consensusRpcUrl = cfgWithDefaults.consensusRpcUrl;
          checkpoint = cfgWithDefaults.checkpoint;
        };
      };
    };

  supportedServiceNames = [
    "postgres"
    "nginx"
    "minio"
    "reth"
    "helios"
  ];

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
  inherit supportedServiceNames;

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
