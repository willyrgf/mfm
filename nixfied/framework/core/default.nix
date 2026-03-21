{
  pkgs,
  system,
}:
{
  canonical = import ./canonical.nix { inherit (pkgs) lib; };

  mkCompiledCore =
    args:
    import ./mkCompiledCore.nix (
      {
        inherit
          pkgs
          system
          ;
      }
      // args
    );

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

  materializeExecution =
    args:
    import ./materializeExecution.nix (
      {
        inherit
          pkgs
          ;
      }
      // args
    );
}
