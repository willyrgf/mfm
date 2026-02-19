{
  pkgs,
  projectRoot,
  registry,
}:
{
  mkApps =
    {
      model,
    }:
    import ./dispatcher.nix {
      inherit
        pkgs
        model
        projectRoot
        registry
        ;
    };
}
