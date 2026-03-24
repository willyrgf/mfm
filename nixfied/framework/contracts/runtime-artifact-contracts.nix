{ pkgs }:
let
  lib = pkgs.lib;
  canonical = import ../core/canonical.nix { inherit lib; };
  contracts = import ../../contracts {
    inherit
      lib
      canonical
      ;
  };
  compileContractBundle = import ../../compiler/compile-contract-bundle.nix {
    inherit
      lib
      canonical
      contracts
      ;
  };
  frameworkDefinitions = import ./default-definitions.nix {
    contracts = contracts.types;
  };
in
compileContractBundle {
  resolved = { };
  inherit frameworkDefinitions;
  defaultVersion = 1;
}
