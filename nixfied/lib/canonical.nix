{ lib ? null }:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);

  renderAttrName = name:
    if builtins.match "[A-Za-z_][A-Za-z0-9_'-]*" name != null then name else builtins.toJSON name;

  canonicalize =
    value:
    let
      valueType = builtins.typeOf value;
    in
    if valueType == "set" then
      builtins.listToAttrs (
        map (
          name: {
            inherit name;
            value = canonicalize value.${name};
          }
        ) (sortNames value)
      )
    else if valueType == "list" then
      map canonicalize value
    else if valueType == "path" then
      builtins.toString value
    else if valueType == "lambda" then
      throw "toCanonicalNix: functions are not allowed in canonicalized values"
    else
      value;

  render =
    value:
    let
      valueType = builtins.typeOf value;
    in
    if valueType == "set" then
      let
        names = sortNames value;
        fields = map (name: "${renderAttrName name} = ${render value.${name}};") names;
      in
      "{ ${builtins.concatStringsSep " " fields} }"
    else if valueType == "list" then
      "[ ${builtins.concatStringsSep " " (map render value)} ]"
    else if valueType == "string" then
      builtins.toJSON value
    else if valueType == "int" || valueType == "float" then
      builtins.toString value
    else if valueType == "bool" then
      if value then "true" else "false"
    else if valueType == "null" then
      "null"
    else if valueType == "path" then
      builtins.toJSON (builtins.toString value)
    else if valueType == "lambda" then
      throw "toCanonicalNix: functions are not allowed in canonicalized values"
    else
      throw "toCanonicalNix: unsupported value type '${valueType}'";

  toCanonicalNix = value: render (canonicalize value);
  hashCanonical = value: builtins.hashString "sha256" (toCanonicalNix value);
in
{
  inherit
    canonicalize
    toCanonicalNix
    hashCanonical
    ;
}
