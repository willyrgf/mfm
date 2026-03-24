{
  lib,
  canonical,
  types,
}:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);

  renderScalar =
    value:
    let
      valueType = builtins.typeOf value;
    in
    if valueType == "string" || valueType == "int" || valueType == "float" then
      value
    else if valueType == "bool" || valueType == "null" then
      value
    else
      throw "render-json-schema: unsupported scalar type '${valueType}'";

  enumPrimitiveType =
    value:
    let
      valueType = builtins.typeOf value;
    in
    if valueType == "string" then
      "string"
    else if valueType == "int" then
      "integer"
    else if valueType == "bool" then
      "boolean"
    else if valueType == "null" then
      "null"
    else
      throw "render-json-schema: unsupported enum primitive '${valueType}'";

  jsonPointerEscape = value: builtins.replaceStrings [ "~" "/" ] [ "~0" "~1" ] value;

  renderStringSchema =
    node:
    {
      type = "string";
    }
    // lib.optionalAttrs (node ? minLength) { minLength = node.minLength; }
    // lib.optionalAttrs (node ? maxLength) { maxLength = node.maxLength; }
    // lib.optionalAttrs (node ? pattern) { pattern = node.pattern; }
    // lib.optionalAttrs (node ? doc) { description = node.doc; };

  renderNode =
    bundle: node:
    let
      baseDescription = lib.optionalAttrs (node ? doc) { description = node.doc; };
    in
    if node.kind == "bool" then
      baseDescription // { type = "boolean"; }
    else if node.kind == "string" then
      renderStringSchema node
    else if node.kind == "integer" then
      {
        type = "integer";
      }
      // lib.optionalAttrs (node ? minimum) { minimum = node.minimum; }
      // lib.optionalAttrs (node ? maximum) { maximum = node.maximum; }
      // baseDescription
    else if node.kind == "number" then
      {
        type = "number";
      }
      // lib.optionalAttrs (node ? minimum) { minimum = node.minimum; }
      // lib.optionalAttrs (node ? maximum) { maximum = node.maximum; }
      // baseDescription
    else if node.kind == "null" then
      baseDescription // { type = "null"; }
    else if node.kind == "literal" then
      baseDescription // { const = renderScalar node.value; }
    else if node.kind == "enum" then
      baseDescription
      // {
        type = enumPrimitiveType (builtins.elemAt node.values 0);
        enum = map renderScalar node.values;
      }
    else if node.kind == "list" then
      {
        type = "array";
        items = renderNode bundle node.elem;
      }
      // lib.optionalAttrs (node ? minItems) { minItems = node.minItems; }
      // lib.optionalAttrs (node ? maxItems) { maxItems = node.maxItems; }
      // baseDescription
    else if node.kind == "map" then
      {
        type = "object";
        additionalProperties = renderNode bundle node.value;
      }
      //
        lib.optionalAttrs
          ((node.key ? minLength) || (node.key ? maxLength) || (node.key ? pattern) || (node.key ? doc))
          {
            propertyNames = renderStringSchema node.key;
          }
      // baseDescription
    else if node.kind == "record" then
      let
        fieldNames = sortNames node.fields;
        properties = builtins.listToAttrs (
          map (
            fieldName:
            let
              field = node.fields.${fieldName};
            in
            {
              name = fieldName;
              value =
                renderNode bundle field.schema // lib.optionalAttrs (field ? doc) { description = field.doc; };
            }
          ) fieldNames
        );
        required = map (fieldName: fieldName) (
          builtins.filter (fieldName: node.fields.${fieldName}.required) fieldNames
        );
      in
      {
        type = "object";
        additionalProperties = !(node.closed);
        inherit properties;
      }
      // lib.optionalAttrs (required != [ ]) { inherit required; }
      // baseDescription
    else if node.kind == "union" then
      baseDescription
      // {
        oneOf = map (option: renderNode bundle option) node.options;
      }
    else if node.kind == "taggedUnion" then
      baseDescription
      // {
        oneOf = map (variantName: renderNode bundle node.variants.${variantName}) (sortNames node.variants);
      }
    else if node.kind == "ref" then
      { "$ref" = "#/$defs/${jsonPointerEscape node.name}"; }
    else
      throw "render-json-schema: unsupported node kind '${node.kind}'";

  renderDocumentAttr =
    bundle: name:
    let
      node = bundle.definitions.${name} or (throw "render-json-schema: unknown definition '${name}'");
      definitionNames = sortNames bundle.definitions;
      defs = builtins.listToAttrs (
        map (definitionName: {
          name = definitionName;
          value = renderNode bundle bundle.definitions.${definitionName};
        }) definitionNames
      );
    in
    canonical.canonicalize (
      {
        "$schema" = "https://json-schema.org/draft/2020-12/schema";
        "$id" = "nixfied:contract:${name}:v${builtins.toString bundle.version}";
        "$ref" = "#/$defs/${jsonPointerEscape name}";
        "$defs" = defs;
        title = name;
      }
      // lib.optionalAttrs (node ? doc) { description = node.doc; }
    );

  renderJson =
    level: value:
    let
      valueType = builtins.typeOf value;
      currentIndent = builtins.concatStringsSep "" (builtins.genList (_: "  ") level);
      nextIndent = builtins.concatStringsSep "" (builtins.genList (_: "  ") (level + 1));
    in
    if valueType == "set" then
      let
        names = sortNames value;
        entries = map (
          name: "${nextIndent}${builtins.toJSON name}: ${renderJson (level + 1) value.${name}}"
        ) names;
      in
      if entries == [ ] then "{}" else "{\n${builtins.concatStringsSep ",\n" entries}\n${currentIndent}}"
    else if valueType == "list" then
      let
        items = map (item: "${nextIndent}${renderJson (level + 1) item}") value;
      in
      if items == [ ] then "[]" else "[\n${builtins.concatStringsSep ",\n" items}\n${currentIndent}]"
    else if valueType == "string" then
      builtins.toJSON value
    else if valueType == "int" || valueType == "float" then
      builtins.toString value
    else if valueType == "bool" then
      if value then "true" else "false"
    else if valueType == "null" then
      "null"
    else
      throw "render-json-schema: unsupported JSON value type '${valueType}'";

  renderDocuments =
    rawBundle:
    let
      bundle = types.normalizeBundle rawBundle;
      definitionNames = sortNames bundle.definitions;
    in
    canonical.canonicalize (
      builtins.listToAttrs (
        map (name: {
          inherit name;
          value = renderDocumentAttr bundle name;
        }) definitionNames
      )
    );
in
{
  document =
    {
      bundle,
      name,
    }:
    renderDocumentAttr (types.normalizeBundle bundle) name;

  documents = renderDocuments;

  definition =
    {
      bundle,
      name,
    }:
    renderJson 0 (renderDocumentAttr (types.normalizeBundle bundle) name) + "\n";

  bundle = rawBundle: renderJson 0 (renderDocuments rawBundle) + "\n";
}
