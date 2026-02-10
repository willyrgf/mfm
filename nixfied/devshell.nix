# Development shell configuration (generic)
{ pkgs, project }:

let
  packages = project.tooling.devShellPackages or [ ];
  hook = project.tooling.devShellHook or "";
  shellName = "${project.project.id or "project"}-dev";
in
pkgs.mkShell {
  name = shellName;
  buildInputs = packages;
  shellHook = hook;
}
