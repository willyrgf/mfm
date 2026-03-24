{
  lib,
  canonical,
  contracts,
}:
{
  resolved,
  frameworkDefinitions ? { },
  defaultVersion ? 1,
}:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);

  rawContracts = resolved.contracts or { };
  version =
    if rawContracts ? version then
      if builtins.typeOf rawContracts.version == "int" then
        rawContracts.version
      else
        throw "nixfied contracts: version must be an integer"
    else
      defaultVersion;
  projectDefinitions =
    if rawContracts ? definitions then
      if builtins.typeOf rawContracts.definitions == "set" then
        rawContracts.definitions
      else
        throw "nixfied contracts: definitions must be an attrset"
    else
      { };

  definitions = frameworkDefinitions // projectDefinitions;
  hasDefinitions = definitions != { };

  normalizedBundle =
    if hasDefinitions then
      contracts.types.normalizeBundle {
        inherit
          version
          definitions
          ;
      }
    else
      canonical.canonicalize {
        inherit version;
        definitions = { };
      };

  definitionNames = sortNames normalizedBundle.definitions;

  cueDefinitions =
    if hasDefinitions then
      builtins.listToAttrs (
        map (name: {
          inherit name;
          value = contracts.renderCue.definition {
            bundle = normalizedBundle;
            inherit name;
          };
        }) definitionNames
      )
    else
      { };

  jsonSchemaDocuments =
    if hasDefinitions then contracts.renderJsonSchema.documents normalizedBundle else { };

  validationSchemas =
    if hasDefinitions then
      builtins.listToAttrs (
        map (name: {
          inherit name;
          value = jsonSchemaDocuments.${name}."$defs".${name};
        }) definitionNames
      )
    else
      { };

  cueBundle =
    if hasDefinitions then contracts.renderCue.bundle normalizedBundle else "package nixfied\n";

  docs =
    if hasDefinitions then
      contracts.renderDocs.bundle normalizedBundle
    else
      builtins.concatStringsSep "\n" [
        "# Nixfied Contract Bundle"
        ""
        "Version: `${builtins.toString version}`"
        ""
      ];
in
canonical.canonicalize {
  bundle = normalizedBundle;
  inherit
    definitionNames
    cueDefinitions
    cueBundle
    jsonSchemaDocuments
    validationSchemas
    docs
    ;
}
