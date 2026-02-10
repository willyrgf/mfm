{
  pkgs ? null,
}:

let
  conf = import ./conf.nix { inherit pkgs; };
  project = conf.project or { };
  parts = [
    conf
    (import ./dev.nix { inherit pkgs project; })
    (import ./test.nix { inherit pkgs project; })
    (import ./prod.nix { inherit pkgs project; })
    (import ./quality.nix { inherit pkgs project; })
    (import ./ci.nix { inherit pkgs project; })
  ];
in
pkgs.lib.foldl' pkgs.lib.recursiveUpdate { } parts
