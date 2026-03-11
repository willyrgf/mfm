{
  pkgs,
  projectRoot,
  registry,
}:
{
  mkApps =
    {
      model,
      serviceRuntime ? { },
    }:
    import ./dispatcher.nix {
      inherit
        pkgs
        model
        projectRoot
        registry
        serviceRuntime
        ;
    };
}
