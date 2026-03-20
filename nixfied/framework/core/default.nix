{
  pkgs,
  system,
}:
{
  canonical = import ./canonical.nix { inherit (pkgs) lib; };

  mkNixfied =
    args:
    import ./mkNixfied.nix (
      {
        inherit
          pkgs
          system
          ;
      }
      // args
    );

  mkFlakeOutputs =
    args:
    import ./mkFlakeOutputs.nix (
      {
        inherit
          pkgs
          system
          ;
      }
      // args
    );
}
