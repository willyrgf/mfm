# Supervisor YAML generation from service definitions
{
  pkgs,
  project,
  slots,
}:

let
  services = project.supervisor.services or { };
  serviceNames = builtins.attrNames services;

  indent =
    level: text:
    let
      spaces = builtins.concatStringsSep "" (builtins.genList (_: " ") level);
      lines = pkgs.lib.splitString "\n" text;
    in
    pkgs.lib.concatMapStringsSep "\n" (line: "${spaces}${line}") lines;

  envBlock =
    env:
    if env == { } then
      ""
    else
      ''
        environment:
      ''
      + pkgs.lib.concatMapStringsSep "\n" (key: "      - ${key}=${toString env.${key}}") (
        builtins.attrNames env
      );

  dependsBlock =
    deps:
    if deps == [ ] then
      ""
    else
      ''
        depends_on:
      ''
      + pkgs.lib.concatMapStringsSep "\n" (dep: ''
        ${dep}:
          condition: process_healthy
      '') deps;

  readinessBlock =
    readiness:
    if readiness == null then
      ""
    else
      let
        initialDelay = toString (readiness.initialDelaySeconds or 2);
        period = toString (readiness.periodSeconds or 5);
        timeout = toString (readiness.timeoutSeconds or 5);
        failure = toString (readiness.failureThreshold or 3);
      in
      if readiness.type or "http" == "exec" then
        ''
          readiness_probe:
            exec:
              command: ${readiness.command or "true"}
            initial_delay_seconds: ${initialDelay}
            period_seconds: ${period}
            timeout_seconds: ${timeout}
            failure_threshold: ${failure}
        ''
      else
        ''
          readiness_probe:
            http_get:
              host: ${readiness.host or "127.0.0.1"}
              port: ${readiness.port or "80"}
              path: ${readiness.path or "/"}
            initial_delay_seconds: ${initialDelay}
            period_seconds: ${period}
            timeout_seconds: ${timeout}
            failure_threshold: ${failure}
        '';

  availabilityBlock =
    availability:
    if availability == null then
      ""
    else
      ''
        availability:
          restart: ${availability.restart or "on_failure"}
          max_restarts: ${toString (availability.maxRestarts or 3)}
          backoff_seconds: ${toString (availability.backoffSeconds or 5)}
      '';

  shutdownBlock =
    shutdown:
    if shutdown == null then
      ""
    else
      ''
        shutdown:
          signal: ${toString (shutdown.signal or 15)}
          timeout_seconds: ${toString (shutdown.timeoutSeconds or 15)}
          ${pkgs.lib.optionalString (shutdown.command or "" != "") "command: ${shutdown.command}"}
      '';

  serviceYaml =
    name: cfg:
    let
      cmd = cfg.command or "";
      workingDir = cfg.workingDir or ".";
      env = cfg.env or { };
      deps = cfg.dependsOn or [ ];
      readiness = cfg.readiness or null;
      availability = cfg.availability or null;
      shutdown = cfg.shutdown or null;
    in
    ''
            ${name}:
              command: |
      ${indent 10 cmd}
              working_dir: ${workingDir}
      ${envBlock env}
      ${readinessBlock readiness}
      ${dependsBlock deps}
      ${availabilityBlock availability}
      ${shutdownBlock shutdown}
    '';

  servicesYaml =
    if serviceNames == [ ] then
      "  # No services configured"
    else
      pkgs.lib.concatMapStringsSep "\n" (name: serviceYaml name services.${name}) serviceNames;

  generateConfig = pkgs.writeShellScript "supervisor-generate-config" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    mkdir -p "$LOG_DIR" "$RUN_DIR" "$CONFIG_DIR"
    chmod 700 "$LOG_DIR" "$RUN_DIR" "$CONFIG_DIR"

    CONFIG_FILE="$CONFIG_DIR/process-compose.yaml"

    cat > "$CONFIG_FILE" << EOF
    version: "0.5"
    log_level: info
    log_location: $LOG_DIR/supervisor.log

    processes:
    ${servicesYaml}
    EOF

    echo "$CONFIG_FILE"
  '';

in
{
  inherit generateConfig servicesYaml;
}
