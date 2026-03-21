{ lib }:
{ serviceCatalog }:
let
  catalog = if serviceCatalog == null then { } else serviceCatalog;
  surfaceDefinitions = import ../framework/runtime/services/public-surface.nix;

  enabledServiceNames = builtins.sort builtins.lessThan (
    lib.unique (
      builtins.map (
        serviceId:
        let
          service = catalog.${serviceId};
        in
        service.name or serviceId
      ) (builtins.filter (serviceId: catalog.${serviceId}.enable or false) (builtins.attrNames catalog))
    )
  );

  requireSurfaceDefinition =
    serviceName:
    if builtins.hasAttr serviceName surfaceDefinitions then
      surfaceDefinitions.${serviceName}
    else
      throw "nixfied.compile-service-surface-catalog: missing public surface definition for service '${serviceName}'";

  appEntries = builtins.concatLists (
    map (
      serviceName:
      let
        serviceSurface = requireSurfaceDefinition serviceName;
      in
      map (
        app:
        let
          opName = app.opName or "";
          appName =
            if (app.appName or "") != "" then
              app.appName
            else if opName != "" then
              "svc::${serviceName}::${opName}"
            else
              throw "nixfied.compile-service-surface-catalog: service '${serviceName}' surface entry is missing opName";
        in
        {
          inherit
            serviceName
            appName
            ;
          opName = opName;
        }
      ) (serviceSurface.apps or [ ])
    ) enabledServiceNames
  );

  appServiceByName = builtins.foldl' (
    acc: entry:
    if builtins.hasAttr entry.appName acc then
      throw "nixfied.compile-service-surface-catalog: duplicate public service app '${entry.appName}'"
    else
      acc
      // {
        ${entry.appName} = entry.serviceName;
      }
  ) { } appEntries;
in
{
  appNames = builtins.sort builtins.lessThan (builtins.attrNames appServiceByName);
  inherit appServiceByName;
}
