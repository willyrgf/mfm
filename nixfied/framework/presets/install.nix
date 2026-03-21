{
  mkCommandTask,
  pkgs,
  frameworkSourceRevision ? "unknown",
  ownerFile ? "nixfied/framework/presets/install.nix",
}:
let
  frameworkInstallRuntimeInputs = [
    pkgs.coreutils
    pkgs.findutils
    pkgs.gawk
    pkgs.git
    pkgs.gnused
    pkgs.rsync
  ];

  frameworkInstallContractArgs = [
    {
      name = "vendor";
      kind = "flag";
      long = "--vendor";
      description = "Generate a vendored wrapper flake.";
    }
    {
      name = "target";
      kind = "option";
      long = "--target";
      type = "string";
      description = "Output directory for generated wrapper.";
    }
    {
      name = "upgrade";
      kind = "flag";
      long = "--upgrade";
      description = "Upgrade vendored framework files in-place and preserve nixfied/project + nixfied/local.";
    }
    {
      name = "reset-project";
      kind = "flag";
      long = "--reset-project";
      description = "When vendoring, overwrite nixfied/project.";
    }
    {
      name = "reset-local";
      kind = "flag";
      long = "--reset-local";
      description = "When vendoring, overwrite nixfied/local.";
    }
  ];

  frameworkUpgradeContractArgs = [
    {
      name = "vendor";
      kind = "flag";
      long = "--vendor";
      description = "Generate a vendored wrapper flake (default for framework::upgrade).";
    }
    {
      name = "target";
      kind = "option";
      long = "--target";
      type = "string";
      description = "Output directory for generated wrapper.";
    }
    {
      name = "reset-project";
      kind = "flag";
      long = "--reset-project";
      description = "When vendoring, overwrite nixfied/project.";
    }
    {
      name = "reset-local";
      kind = "flag";
      long = "--reset-local";
      description = "When vendoring, overwrite nixfied/local.";
    }
  ];

  mkFrameworkInstallCommand = import ../install/wrapper-command.nix {
    inherit
      pkgs
      frameworkSourceRevision
      ;
    sourceRoot = ../../.;
    repoRoot = ../../../.;
  };
in
{
  tasks = {
    framework-install = mkCommandTask {
      id = "task.framework.install";
      appName = "framework::install";
      kind = "utility";
      summary = "Install thin or vendored wrapper flake";
      description = "Creates a thin wrapper flake by default, or a vendored wrapper with --vendor. Re-running with --vendor preserves nixfied/project and nixfied/local by default.";
      runtimeInputs = frameworkInstallRuntimeInputs;
      usage = [
        "nix run .#framework::install"
        "nix run .#framework::install -- --vendor"
        "nix run .#framework::install -- --vendor --target ."
        "nix run .#framework::install -- --vendor --upgrade --target ."
      ];
      contractArgs = frameworkInstallContractArgs;
      command = mkFrameworkInstallCommand { };
      inherit ownerFile;
    };

    framework-upgrade = mkCommandTask {
      id = "task.framework.upgrade";
      appName = "framework::upgrade";
      kind = "utility";
      summary = "Upgrade vendored wrapper in-place";
      description = ''
        Upgrades framework files while preserving nixfied/project and nixfied/local by default.
        Use --reset-project/--reset-local to overwrite those paths.
        Do not target a framework workspace root itself; use a downstream repo or another target path.
      '';
      runtimeInputs = frameworkInstallRuntimeInputs;
      usage = [
        "nix run .#framework::upgrade -- --target ."
        "nix run .#framework::upgrade -- --target . --reset-project"
        "nix run .#framework::upgrade -- --target . --reset-local"
      ];
      contractArgs = frameworkUpgradeContractArgs;
      command = mkFrameworkInstallCommand {
        upgradeDefault = true;
      };
      inherit ownerFile;
    };
  };
}
