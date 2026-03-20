{
  pkgs,
  projectRoot,
  registry,
}:
{
  mkApps =
    {
      model,
      services,
      runtimeHash ? model.identity.evalHash,
      frameworkSourceFlakeRef ? null,
      serviceApps ? { },
      serviceHookEnv ? { },
    }:
    import ./dispatcher.nix {
      inherit
        pkgs
        model
        services
        runtimeHash
        projectRoot
        registry
        frameworkSourceFlakeRef
        serviceApps
        serviceHookEnv
        ;
    };
}
