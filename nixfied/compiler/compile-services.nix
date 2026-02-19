{ lib }:
{ resolved }:
let
  services = resolved.services or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames services);
in
builtins.listToAttrs (
  map (name: {
    name = "service.${name}";
    value = {
      id = "service.${name}";
      name = name;
      enable = services.${name}.enable or false;
      config = lib.removeAttrs services.${name} [ "enable" ];
    };
  }) names
)
