{
  sourcePath,
  metadataPath ? null,
  self ? null,
}:
let
  dirtyRev = if self != null && self ? dirtyRev then self.dirtyRev else null;

  rev = if self != null && self ? rev then self.rev else null;

  lines =
    if metadataPath != null && builtins.pathExists metadataPath then
      builtins.filter builtins.isString (builtins.split "\n" (builtins.readFile metadataPath))
    else
      [ ];

  findPersistedRevision =
    remaining:
    if remaining == [ ] then
      null
    else if builtins.head remaining == "Framework source revision (install/upgrade):" then
      let
        afterHeader = builtins.tail remaining;
      in
      if afterHeader == [ ] then
        null
      else
        let
          revisionLine = builtins.head afterHeader;
          hasPrefix = builtins.stringLength revisionLine >= 2 && builtins.substring 0 2 revisionLine == "- ";
          candidate =
            if hasPrefix then
              builtins.substring 2 (builtins.stringLength revisionLine - 2) revisionLine
            else
              "";
        in
        if
          !hasPrefix
          || candidate == ""
          || candidate == "unknown"
          || candidate == "set by `framework::install` / `framework::upgrade`"
        then
          null
        else
          candidate
    else
      findPersistedRevision (builtins.tail remaining);

  persistedRevision = findPersistedRevision lines;

  fallbackRevision = builtins.substring 0 12 (
    builtins.hashString "sha256" (
      builtins.toString (
        builtins.path {
          path = sourcePath;
          name = "nixfied-framework-source";
        }
      )
    )
  );
in
if dirtyRev != null then
  dirtyRev
else if rev != null then
  rev
else if persistedRevision != null then
  persistedRevision
else
  fallbackRevision
