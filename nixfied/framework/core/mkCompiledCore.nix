{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
  frameworkSourceRevision ? import ./framework-revision.nix {
    sourcePath = ../../.;
    metadataPath = ../../VENDORED.txt;
  },
}:
let
  lib = pkgs.lib;
  modules = import ../../modules;
  canonical = import ./canonical.nix { inherit lib; };

  compiler = import ../../compiler {
    inherit
      pkgs
      canonical
      modules
      system
      projectRoot
      frameworkSourceRevision
      ;
  };

  compiled = compiler.compileCore {
    inherit
      projectModules
      extraModules
      localOverrides
      ;
  };
in
compiled
