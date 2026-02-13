# Shared validation predicates for API contract checks.
{ pkgs }:

{
  isNonEmptyString = x: builtins.isString x && x != "";
  isNonEmptyList = x: builtins.isList x && x != [ ];
  expect = cond: msg: if cond then [ ] else [ msg ];
  isListOfNonEmptyStrings =
    xs: builtins.isList xs && builtins.all (x: builtins.isString x && x != "") xs;
  isKVSpec =
    x:
    builtins.isAttrs x
    && builtins.isString (x.name or "")
    && (x.name or "") != ""
    && builtins.isString (x.description or "")
    && (x.description or "") != "";
}
