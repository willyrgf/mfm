{ lib }:
let
  serviceConfig = import ../framework/core/service-config.nix { inherit lib; };
in
{ resolved }:
let
  services = resolved.services or { };
  excludedServices = resolved.graph.excludedServices or [ ];
  names = builtins.sort builtins.lessThan (
    builtins.filter (name: !(builtins.elem name excludedServices)) (builtins.attrNames services)
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
        discardContext = true;
        inherit name;
        config = lib.removeAttrs services.${name} [ "enable" ];
      };
    };
  }) names
)
