{ lib, canonical }:
let
  sanitize =
    raw:
    let
      lowered = lib.toLower (builtins.replaceStrings [ " " ] [ "-" ] raw);
      cleaned = builtins.replaceStrings [ "/" ":" "::" "." ] [ "-" "-" "-" "-" ] lowered;
      safe = builtins.replaceStrings [ "--" ] [ "-" ] cleaned;
    in
    if safe == "" then "item" else safe;

  ensurePrefix = prefix: id: if lib.hasPrefix "${prefix}." id then id else "${prefix}.${id}";

in
{
  inherit
    sanitize
    ensurePrefix
    ;
}
