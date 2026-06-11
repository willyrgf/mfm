{
  description = "Nixfied project";

  inputs = {
    nixfied.url = "github:willyrgf/nixfied?ref=v2";
  };

  outputs =
    { self, nixfied }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        builtins.listToAttrs (
          map (system: {
            name = system;
            value = f system;
          }) systems
        );
    in
    {
      packages = forAllSystems (system: {
        default = self.packages.${system}.model;
        model = nixfied.lib.${system}.compileModel ./nixfied.nix;
      });

      apps = forAllSystems (system: nixfied.lib.${system}.projectApps ./nixfied.nix);
    };
}
