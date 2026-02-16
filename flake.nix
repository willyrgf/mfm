{
  description = "Nixfied framework";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        project = import ./nixfied/project { inherit pkgs; };
        slots = import ./nixfied/.framework/slots.nix { inherit pkgs project; };

        postgres =
          if (project.modules.postgres.enable or false) then
            import ./nixfied/.framework/postgres { inherit pkgs project slots; }
          else
            null;

        nginx =
          if (project.modules.nginx.enable or false) then
            import ./nixfied/.framework/nginx { inherit pkgs project slots; }
          else
            null;

        minio =
          if (project.modules.minio.enable or false) then
            import ./nixfied/.framework/minio { inherit pkgs project slots; }
          else
            null;

        reth =
          if (project.modules.reth.enable or false) then
            import ./nixfied/.framework/reth { inherit pkgs project slots; }
          else
            null;

        helios =
          if (project.modules.helios.enable or false) then
            import ./nixfied/.framework/helios { inherit pkgs project slots; }
          else
            null;

        serviceApiLib = import ./nixfied/.framework/lib/service-api.nix { inherit pkgs; };
        serviceApis =
          (pkgs.lib.optionalAttrs (postgres != null) { postgres = postgres.publicApi or null; })
          // (pkgs.lib.optionalAttrs (nginx != null) { nginx = nginx.publicApi or null; })
          // (pkgs.lib.optionalAttrs (minio != null) { minio = minio.publicApi or null; })
          // (pkgs.lib.optionalAttrs (reth != null) { reth = reth.publicApi or null; })
          // (pkgs.lib.optionalAttrs (helios != null) { helios = helios.publicApi or null; });
        enabledServices = builtins.attrNames serviceApis;
        serviceApisValidated = serviceApiLib.validateEnabledServicesHaveContracts {
          enabledServices = enabledServices;
          serviceApis = serviceApis;
        };

        ephemeral =
          if (project.ephemeral.enable or false) then
            import ./nixfied/.framework/ephemeral.nix { inherit pkgs project; }
          else
            null;

        hooks = import ./nixfied/.framework/hooks.nix {
          inherit
            pkgs
            project
            slots
            postgres
            nginx
            minio
            reth
            helios
            supervisor
            ephemeral
            ;
          serviceApis = serviceApisValidated;
        };

        lib = import ./nixfied/.framework/lib { inherit pkgs project hooks; };
        supervisor =
          if (project.supervisor.enable or true) then
            import ./nixfied/.framework/supervisor { inherit pkgs project slots; }
          else
            null;

        coreApps = import ./nixfied/.framework/internal/core.nix {
          inherit
            pkgs
            project
            lib
            moduleApps
            ;
        };
        isFramework = builtins.pathExists ./nixfied/.framework/.workspace;
        frameworkRevision =
          let
            dirtyRev = if self ? dirtyRev then self.dirtyRev else null;
            rev = if self ? rev then self.rev else null;
          in
          if dirtyRev != null then
            dirtyRev
          else if rev != null then
            rev
          else
            "unknown";

        installApps =
          if isFramework then
            import ./nixfied/.framework/internal/install.nix {
              inherit
                pkgs
                lib
                ;
              frameworkRoot = ./.;
              inherit frameworkRevision;
            }
          else
            { };

        testApps =
          if isFramework then
            import ./nixfied/.framework/internal/test.nix {
              inherit
                pkgs
                lib
                ;
            }
          else
            { };
        knownAppNames = pkgs.lib.unique (
          (builtins.attrNames (project.commands or { }))
          ++ (builtins.attrNames moduleApps)
          ++ [
            "help"
            "ci"
            "validate-env"
            "test-isolation"
          ]
        );
        isolationApps = import ./nixfied/.framework/internal/isolation.nix {
          inherit
            pkgs
            project
            lib
            slots
            ;
          knownApps = knownAppNames;
        };
        moduleApps = import ./nixfied/.framework/internal/module-apps.nix {
          inherit
            pkgs
            project
            lib
            supervisor
            slots
            ;
          serviceApis = serviceApisValidated;
        };
        frameworkApps = pkgs.lib.mapAttrs' (name: value: {
          name = "framework::${name}";
          value = value;
        }) (installApps // testApps);
        ciEntry = import ./nixfied/.framework/ci.nix {
          inherit
            pkgs
            project
            lib
            ephemeral
            ;
          knownApps = knownAppNames;
        };
        ciApp =
          if ciEntry == null then
            null
          else if ciEntry ? app then
            ciEntry.app
          else
            ciEntry;

        local =
          if builtins.pathExists ./nixfied/local then
            import ./nixfied/local {
              inherit
                pkgs
                project
                lib
                slots
                hooks
                postgres
                nginx
                minio
                reth
                helios
                supervisor
                ephemeral
                ;
            }
          else
            { };
      in
      {
        devShells = {
          default = import ./nixfied/.framework/devshell.nix {
            inherit
              pkgs
              project
              ;
          };
        }
        // (local.devShells or { });

        apps =
          let
            apps0 =
              coreApps
              // moduleApps
              // (if ciApp != null then { ci = ciApp; } else { })
              // isolationApps
              // frameworkApps
              // (local.apps or { })
              // {
                default = if coreApps ? help then coreApps.help else coreApps.dev;
              };
          in
          lib.appApi.validateApps apps0;

        packages = pkgs.lib.recursiveUpdate (project.packages or { }) (local.packages or { });
      }
    );
}
