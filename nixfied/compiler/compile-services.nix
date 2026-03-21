{ lib }:
{
  resolved,
  pkgs,
  selectedServices ? null,
  discardContext ? true,
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
  map (name: {
    name = "service.${name}";
      value = {
        id = "service.${name}";
        name = name;
        enable = services.${name}.enable or false;
        config = serviceConfig.normalizeServiceConfig {
          inherit discardContext;
          inherit name;
          config = lib.removeAttrs services.${name} [ "enable" ];
        };
      };
  }) names
)
