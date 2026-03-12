let
  canonicalRelativePath = ".workspace";
in
{
  inherit canonicalRelativePath;

  relativePaths = [ canonicalRelativePath ];

  isPresent =
    projectRoot:
    let
      projectRootPath = builtins.toString projectRoot;
    in
    builtins.pathExists "${projectRootPath}/${canonicalRelativePath}";
}
