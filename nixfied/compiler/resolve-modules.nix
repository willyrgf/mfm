{ lib, modules }:
{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
}:
let
  evaluated = lib.evalModules {
    modules =
      [
        {
          imports = [ modules.core ];
        }
      ]
      ++ projectModules
      ++ extraModules
      ++ localOverrides;

    specialArgs = {
      inherit
        pkgs
        system
        projectRoot
        modules
        ;
    };
  };
in
{
  config = evaluated.config.nixfied;
  fullConfig = evaluated.config;
  options = evaluated.options;
}
