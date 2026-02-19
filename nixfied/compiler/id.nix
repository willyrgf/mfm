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

  stableSuffix = payload: builtins.substring 0 10 (canonical.hashCanonical payload);
in
{
  inherit
    sanitize
    ensurePrefix
    stableSuffix
    ;

  taskId =
    {
      name,
      id,
      payload,
    }:
    let
      requested = if id == "" || id == name then "task.${sanitize name}" else ensurePrefix "task" id;
    in
    "${requested}-${stableSuffix payload}";

  workflowId =
    {
      name,
      id,
      payload,
    }:
    let
      requested =
        if id == "" || id == name then "workflow.${sanitize name}" else ensurePrefix "workflow" id;
    in
    "${requested}-${stableSuffix payload}";
}
