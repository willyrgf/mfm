# Fixture DSL compiler (Nix -> shell prelude)
{
  pkgs,
  project,
}:

let
  lib = pkgs.lib;
  servicePolicy = import ./service-policy.nix { inherit pkgs; };

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
      eval "$(run_hook SVC_MINIO_EXPORT_S3_ENV ${quote bucket} ${quote prefix} ${quote region})"
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
              run_hook SVC_MINIO_BUCKET_ENSURE ${quote bucketName}
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
      logPrefix = if globalArtifacts ? prefix then toString globalArtifacts.prefix else contextName;
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
        _fixture_log_file=""
        ${lib.optionalString logsEnabled ''
          _fixture_log_file="$(artifact_path ${quote logName})"
        ''}
        # Lifecycle contract for fixture services:
        # - start may be asynchronous.
        # - fixture_start_service should prefer profile-specific READY hooks when available.
        # - fixture_start_service must poll READY/HEALTH with timeout.
        # - diagnostics must stay robust when log files are missing.
        _fixture_keep_running="$(_fixture_keep_running_from_policy)"
        log_info "fixture service start name=${serviceName} profile=${profile}"
        fixture_start_service ${quote serviceName} ${quote profile} ${quote timeout} ${quote interval} "$_fixture_log_file" "$_fixture_keep_running"
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
        mkHttpUrlExportFromPortVar key rethHttpPortVar
      else if from == "helios.rpcUrl" then
        mkHttpUrlExportFromPortVar key heliosRpcPortVar
      else if from == "minio.endpoint" then
        mkHttpUrlExportFromPortVar key minioApiPortVar
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

  mkHttpUrlExportFromPortVar = key: portVar: ''
    _fixture_port_var="${portVar}"
    _fixture_port_val="''${!_fixture_port_var:-}"
    export ${key}="http://127.0.0.1:''${_fixture_port_val}"
  '';

  keepRunningFromPolicy = ''
    ${servicePolicy.policyRuntimeFunctions}

    _fixture_keep_running_from_policy() {
      local owner_scope="''${SERVICE_OWNER_SCOPE:-}"
      local reuse_policy="''${SERVICE_REUSE_POLICY:-}"
      local discovery_scope="''${SERVICE_DISCOVERY_SCOPE:-}"

      nixfied_policy_validate_owner_scope "$owner_scope" 1 || return 1
      nixfied_policy_validate_reuse_policy "$reuse_policy" 1 || return 1
      nixfied_policy_validate_discovery_scope "$discovery_scope" 1 || return 1

      case "$owner_scope" in
        persistent) echo "1"; return 0 ;;
        ephemeral) echo "0"; return 0 ;;
        "") ;;
      esac

      case "$reuse_policy" in
        same-slot|cross-run) echo "1"; return 0 ;;
        never|same-root) echo "0"; return 0 ;;
        "") ;;
      esac

      case "$discovery_scope" in
        global) echo "1"; return 0 ;;
        local) echo "0"; return 0 ;;
        "") ;;
      esac

      # Keep default fixture semantics unchanged unless policy envs request persistence.
      echo "0"
      return 0
    }
  '';

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
      keepRunningScript = if services == [ ] then "" else keepRunningFromPolicy;
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
      ++ (if keepRunningScript == "" then [ ] else [ keepRunningScript ])
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
