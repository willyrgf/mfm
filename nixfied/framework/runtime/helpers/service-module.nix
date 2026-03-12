{
  pkgs,
  project,
  slots,
}:

let
  serviceApi = import ./service-api.nix { inherit pkgs; };
  runtimeEvents = import ./runtime-events.nix { inherit pkgs project; };
  observability = import ./service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };

  mkServiceModule =
    {
      service,
      summary,
      details,
      artifacts,
      operations,
      summaryName ? service,
      profiles ? [ ],
      runtimePrimitives ? serviceApi.mkRuntimePrimitivesV1 {
        logLevelDefault = toString ((project.logging or { }).level or "info");
        outputModeDefault = toString ((project.logging or { }).output or "stdout");
      },
      config ? null,
      exported ? { },
    }:
    let
      logs = observability.mkLogScript service;
      log = logs;
      events = observability.mkEventsScript service;
      logEventExtensions = observability.mkLogEventExtensions {
        inherit
          service
          summaryName
          ;
        logScript = log;
        eventsScript = events;
      };
      publicApi = serviceApi.mkServiceApiV3 {
        inherit
          service
          summary
          details
          profiles
          artifacts
          runtimePrimitives
          ;
        operations = operations // logEventExtensions;
      };
    in
    (pkgs.lib.optionalAttrs (config != null) { inherit config; })
    // exported
    // {
      inherit
        publicApi
        log
        logs
        events
        ;
    };
in
{
  inherit mkServiceModule;
}
