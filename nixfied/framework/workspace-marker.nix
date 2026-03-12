let
  canonicalRelativePath = ".workspace";
  legacyRelativePath = "nixfied/.framework/.workspace";
  legacyRelativePaths = [ legacyRelativePath ];
in
{
  inherit
    canonicalRelativePath
    legacyRelativePath
    legacyRelativePaths
    ;

  relativePaths = [ canonicalRelativePath ] ++ legacyRelativePaths;

  isPresent =
    projectRoot:
    let
      safeProjectRoot = builtins.unsafeDiscardStringContext (builtins.toString projectRoot);
    in
    builtins.any (relativePath: builtins.pathExists "${safeProjectRoot}/${relativePath}") (
      [ canonicalRelativePath ] ++ legacyRelativePaths
    );
}
