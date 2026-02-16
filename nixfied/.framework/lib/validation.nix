# Shared validation predicates for API contract checks.
{ pkgs }:

rec {
  isNonEmptyString = x: builtins.isString x && x != "";
  isNonEmptyList = x: builtins.isList x && x != [ ];
  expect = cond: msg: if cond then [ ] else [ msg ];
  renderErrors = errs: builtins.concatStringsSep "\n" (map (e: "  - " + e) errs);
  sortedAttrNames = attrs: pkgs.lib.sort (a: b: a < b) (builtins.attrNames attrs);
  optionalAttrSatisfies = attrs: field: pred: !(builtins.hasAttr field attrs) || pred (attrs.${field});
  isEnvVarName = value: builtins.isString value && (builtins.match "^[A-Z_][A-Z0-9_]*$" value) != null;
  isScalar =
    value:
    builtins.isString value
    || builtins.isInt value
    || builtins.isBool value
    || builtins.isFloat value
    || builtins.isPath value;
  isScalarAttrset =
    value:
    builtins.isAttrs value
    && builtins.all isEnvVarName (builtins.attrNames value)
    && builtins.all (key: isScalar value.${key}) (builtins.attrNames value);
  isListOfNonEmptyStrings =
    xs: builtins.isList xs && builtins.all (x: builtins.isString x && x != "") xs;
  isKVSpec =
    x:
    builtins.isAttrs x
    && builtins.isString (x.name or "")
    && (x.name or "") != ""
    && builtins.isString (x.description or "")
    && (x.description or "") != "";
  isKVSpecList = xs: builtins.isList xs && builtins.all isKVSpec xs;
}
