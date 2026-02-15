{
  pkgs ? null,
}:

let
  conf = import ./conf.nix { inherit pkgs; };
  project = conf.project or { };
  lib = import ../.framework/lib {
    inherit pkgs;
    project = conf;
  };
  parts = [
    conf
    (import ./dev.nix { inherit pkgs project lib; })
    (import ./test.nix { inherit pkgs project lib; })
    (import ./prod.nix { inherit pkgs project lib; })
    (import ./quality.nix { inherit pkgs project lib; })
    (import ./ci.nix { inherit pkgs project lib; })
  ];
in
pkgs.lib.foldl' pkgs.lib.recursiveUpdate { } parts
