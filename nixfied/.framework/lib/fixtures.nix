# Fixture DSL compiler (Nix -> shell prelude)
{
  pkgs,
  project,
}:

let
  lib = pkgs.lib;

  modules = project.modules or { };
  postgresCfg = modules.postgres or { };
  minioCfg = modules.minio or { };
  rethCfg = modules.reth or { };
  heliosCfg = modules.helios or { };
  defaultPostgresDatabase = postgresCfg.testDatabase or (postgresCfg.database or "app_test");

  portVarFromKey =
    key:
    let
      replaced = lib.replaceStrings [ "-" "." ":" "/" " " ] [ "_" "_" "_" "_" "_" ] (toString key);
    in
    lib.strings.toUpper replaced + "_PORT";

  postgresPortVar = portVarFromKey (postgresCfg.portKey or "postgres");
  minioApiPortVar = portVarFromKey (minioCfg.portKeyApi or "minioApi");
  rethHttpPortVar = portVarFromKey (rethCfg.portKeyHttp or "rethHttp");
  heliosRpcPortVar = portVarFromKey (heliosCfg.portKeyRpc or "heliosRpc");

  isEnvName = name: builtins.match "^[A-Za-z_][A-Za-z0-9_]*$" name != null;

  normalizeServiceSpec =
    spec:
    if builtins.isString spec then
      { name = spec; }
    else if builtins.isAttrs spec then
      spec
    else
      throw "Fixture service entry must be a string or attrset";

  assertService =
    serviceSpec:
    let
      name = serviceSpec.name or (throw "Fixture service entry missing `name`");
      cfg = modules.${name} or null;
      enabled = if cfg == null then false else (cfg.enable or false);
    in
    if cfg == null then
      throw "Unknown fixture service: ${name}"
    else if !enabled then
      throw "Fixture service `${name}` requires project.modules.${name}.enable = true"
    else
      serviceSpec;

  quote = v: lib.escapeShellArg (toString v);

  mkMinioExportScript =
    serviceSpec:
    let
      bucket = serviceSpec.bucket or "";
      prefix = serviceSpec.prefix or "";
      region = serviceSpec.region or "us-east-1";
    in
    ''
      eval "$(run_hook MINIO_EXPORT_S3_ENV ${quote bucket} ${quote prefix} ${quote region})"
    '';

  mkBootstrapScript =
    serviceName: bootstrap:
    let
      normalizeAction =
        action:
        if builtins.isString action then
          {
            kind = "bucket";
            name = action;
          }
        else if builtins.isAttrs action then
          action
        else
          throw "Fixture bootstrap action for `${serviceName}` must be string or attrset";

      renderAction =
        action0:
        let
          action = normalizeAction action0;
          kind = action.kind or (throw "Fixture bootstrap action for `${serviceName}` missing `kind`");
        in
        if kind == "bucket" then
          if serviceName != "minio" then
            throw "Fixture bootstrap kind=bucket only supported for service=minio"
          else
            let
              bucketName = action.name or (throw "Fixture bootstrap kind=bucket requires `name`");
            in
            ''
              run_hook MINIO_BUCKET_ENSURE ${quote bucketName}
            ''
        else
          throw "Unsupported fixture bootstrap kind for `${serviceName}`: ${kind}";
    in
    lib.concatMapStringsSep "\n" renderAction bootstrap;

  mkServiceScript =
    {
      contextName,
      defaultProfile,
      defaultLogs,
      globalArtifacts,
    }:
    idx: rawSpec:
    let
      serviceSpec = assertService (normalizeServiceSpec rawSpec);
      serviceName = serviceSpec.name;
      profile = serviceSpec.profile or defaultProfile;
      timeout = toString (serviceSpec.timeout or 60);
      interval = toString (serviceSpec.interval or 1);
      exportsList = serviceSpec.exports or [ ];
      bootstrap = serviceSpec.bootstrap or [ ];
      logsEnabled =
        if serviceSpec ? logs then
          serviceSpec.logs
        else if globalArtifacts ? logs then
          globalArtifacts.logs
        else
          defaultLogs;
      logPrefix =
        if globalArtifacts ? prefix then
          toString globalArtifacts.prefix
        else
          contextName;
      logName =
        if serviceSpec ? logName then
          toString serviceSpec.logName
        else
          "${logPrefix}-${toString idx}-${serviceName}.log";

      exportScript = lib.concatMapStringsSep "\n" (
        exportToken:
        if exportToken == "s3" then
          if serviceName != "minio" then
            throw "Fixture exports=[\"s3\"] is only supported for service=minio"
          else
            mkMinioExportScript serviceSpec
        else
          throw "Unsupported fixture export token `${exportToken}` for service `${serviceName}`"
      ) exportsList;

      bootstrapScript = mkBootstrapScript serviceName bootstrap;
    in
    ''
      {
        local _fixture_log_file=""
        ${lib.optionalString logsEnabled ''
          _fixture_log_file="$(artifact_path ${quote logName})"
        ''}
        echo "INFO: fixture service start name=${serviceName} profile=${profile}"
        fixture_start_service ${quote serviceName} ${quote profile} ${quote timeout} ${quote interval} "$_fixture_log_file"
        ${exportScript}
        ${bootstrapScript}
      }
    '';

  mkEnvExportScript =
    key: value:
    if !isEnvName key then
      throw "Invalid fixtures.env key (must be shell-safe env var name): ${key}"
    else if builtins.isString value || builtins.isInt value || builtins.isBool value then
      "export ${key}=${quote value}"
    else if builtins.isAttrs value then
      let
        from = value.from or (throw "fixtures.env.${key} requires `from` when value is an attrset");
      in
      if from == "postgres.url" then
        let
          dbName = value.database or defaultPostgresDatabase;
        in
        ''
          _fixture_port_var="${postgresPortVar}"
          _fixture_port_val="''${!_fixture_port_var:-}"
          export ${key}="postgresql://postgres:postgres@127.0.0.1:''${_fixture_port_val}/${dbName}"
        ''
      else if from == "reth.httpUrl" then
        ''
          _fixture_port_var="${rethHttpPortVar}"
          _fixture_port_val="''${!_fixture_port_var:-}"
          export ${key}="http://127.0.0.1:''${_fixture_port_val}"
        ''
      else if from == "helios.rpcUrl" then
        ''
          _fixture_port_var="${heliosRpcPortVar}"
          _fixture_port_val="''${!_fixture_port_var:-}"
          export ${key}="http://127.0.0.1:''${_fixture_port_val}"
        ''
      else if from == "minio.endpoint" then
        ''
          _fixture_port_var="${minioApiPortVar}"
          _fixture_port_val="''${!_fixture_port_var:-}"
          export ${key}="http://127.0.0.1:''${_fixture_port_val}"
        ''
      else if from == "minio.bucket" then
        ''
          export ${key}="''${MINIO_BUCKET:-}"
        ''
      else if from == "minio.region" then
        ''
          export ${key}="''${MINIO_REGION:-us-east-1}"
        ''
      else if from == "minio.prefix" then
        ''
          export ${key}="''${MINIO_PREFIX:-}"
        ''
      else if from == "env" then
        let
          varName = value.var or (throw "fixtures.env.${key} with from=\"env\" requires `var`");
        in
        ''
          export ${key}="''${${varName}:-}"
        ''
      else
        throw "Unsupported fixtures.env.${key}.from value: ${from}"
    else
      throw "fixtures.env.${key} must be a scalar or attrset";

  renderPrelude =
    {
      fixtures ? null,
      contextName,
      defaultProfile ? "default",
      defaultLogs ? false,
    }:
    let
      cfg = if fixtures == null then { } else fixtures;
      services = cfg.services or [ ];
      envCfg = cfg.env or { };
      artifactsCfg = cfg.artifacts or { };
      serviceScripts = lib.imap0 (mkServiceScript {
        inherit
          contextName
          defaultProfile
          defaultLogs
          ;
        globalArtifacts = artifactsCfg;
      }) services;
      envExports = lib.concatMapStringsSep "\n" (key: mkEnvExportScript key envCfg.${key}) (
        builtins.attrNames envCfg
      );
    in
    lib.concatStringsSep "\n" (
      [ ]
      ++ (if services == [ ] then [ ] else serviceScripts)
      ++ (if envExports == "" then [ ] else [ envExports ])
    );

  wrapScript =
    {
      script,
      fixtures ? null,
      contextName,
      defaultProfile ? "default",
      defaultLogs ? false,
    }:
    let
      prelude = renderPrelude {
        inherit
          fixtures
          contextName
          defaultProfile
          defaultLogs
          ;
      };
    in
    if prelude == "" then
      script
    else
      ''
        ${prelude}
        ${script}
      '';
in
{
  inherit
    renderPrelude
    wrapScript
    ;
}
