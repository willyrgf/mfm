{ lib }:
{
  resolved,
  pkgs,
  selectedServices ? null,
  enabledServiceFlags ? { },
}:
let
  serviceConfig = import ../framework/core/service-config.nix {
    inherit lib pkgs;
  };
  services = resolved.services or { };
  excludedServices = resolved.graph.excludedServices or [ ];
  selectedServiceNames =
    if selectedServices == null then
      null
    else
      builtins.sort builtins.lessThan (lib.unique (builtins.filter (name: name != "") selectedServices));
  selectedServiceSet =
    if selectedServiceNames == null then
      { }
    else
      builtins.listToAttrs (
        map (serviceName: {
          name = serviceName;
          value = true;
        }) selectedServiceNames
      );
  names = builtins.sort builtins.lessThan (
    builtins.filter (
      name:
      !(builtins.elem name excludedServices)
      && (
        selectedServiceNames == null
        || builtins.hasAttr name selectedServiceSet
        || builtins.hasAttr "service.${name}" selectedServiceSet
      )
    ) (builtins.attrNames services)
  );
in
builtins.listToAttrs (
  map (
    name:
    let
      serviceEnabled = enabledServiceFlags.${name} or (services.${name}.enable or false);
      rawConfig = lib.removeAttrs services.${name} [ "enable" ];
      configInput = if serviceEnabled then rawConfig else rawConfig // { defaultSource = ""; };
    in
    {
      name = "service.${name}";
      value = {
        id = "service.${name}";
        name = name;
        enable = serviceEnabled;
        config = serviceConfig.normalizeServiceConfig {
          discardContext = true;
          inherit name;
          config = configInput;
        };
      };
    }
  ) names
)
