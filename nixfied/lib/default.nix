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
}
