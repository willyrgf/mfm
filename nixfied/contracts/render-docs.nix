{
  lib,
  types,
}:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);

  indent = level: builtins.concatStringsSep "" (builtins.genList (_: "  ") level);

  inlineType =
    node:
    if node.kind == "bool" then
      "bool"
    else if node.kind == "string" then
      "string"
      + lib.optionalString (node ? minLength) " minLength=${builtins.toString node.minLength}"
      + lib.optionalString (node ? maxLength) " maxLength=${builtins.toString node.maxLength}"
      + lib.optionalString (node ? pattern) " pattern=${builtins.toJSON node.pattern}"
    else if node.kind == "integer" then
      "integer"
      + lib.optionalString (node ? minimum) " minimum=${builtins.toString node.minimum}"
      + lib.optionalString (node ? maximum) " maximum=${builtins.toString node.maximum}"
    else if node.kind == "number" then
      "number"
      + lib.optionalString (node ? minimum) " minimum=${builtins.toString node.minimum}"
      + lib.optionalString (node ? maximum) " maximum=${builtins.toString node.maximum}"
    else if node.kind == "null" then
      "null"
    else if node.kind == "literal" then
      "literal ${builtins.toJSON node.value}"
    else if node.kind == "enum" then
      "enum ${builtins.concatStringsSep " | " (map builtins.toJSON node.values)}"
    else if node.kind == "list" then
      "list of ${inlineType node.elem}"
    else if node.kind == "map" then
      "map ${inlineType node.key} -> ${inlineType node.value}"
    else if node.kind == "record" then
      "record closed=${if node.closed then "true" else "false"}"
    else if node.kind == "union" then
      "union"
    else if node.kind == "taggedUnion" then
      "taggedUnion tag=${node.tag}"
    else if node.kind == "ref" then
      "ref ${node.name}"
    else
      node.kind;

  renderNode =
    level: name: node:
    let
      heading = "${indent level}- `${name}`: ${inlineType node}";
      detailLines =
        if node.kind == "record" then
          map (
            fieldName:
            let
              field = node.fields.${fieldName};
              requirement = if field.required then "required" else "optional";
            in
            builtins.concatStringsSep "\n" (
              [
                "${indent (level + 1)}- `${fieldName}` (${requirement}): ${inlineType field.schema}"
              ]
              ++ lib.optionals (field ? doc) [ "${indent (level + 2)}${field.doc}" ]
              ++ lib.optionals (builtins.elem field.schema.kind [
                "record"
                "union"
                "taggedUnion"
              ]) [ (renderNode (level + 2) "${fieldName}.schema" field.schema) ]
            )
          ) (sortNames node.fields)
        else if node.kind == "taggedUnion" then
          map (variantName: renderNode (level + 1) variantName node.variants.${variantName}) (
            sortNames node.variants
          )
        else if node.kind == "union" then
          map (
            index:
            renderNode (level + 1) "option-${builtins.toString index}" (builtins.elemAt node.options index)
          ) (builtins.genList (index: index) (builtins.length node.options))
        else
          [ ];
    in
    builtins.concatStringsSep "\n" (
      [ heading ] ++ lib.optionals (node ? doc) [ "${indent (level + 1)}${node.doc}" ] ++ detailLines
    );
in
{
  bundle =
    rawBundle:
    let
      bundle = types.normalizeBundle rawBundle;
      definitionNames = sortNames bundle.definitions;
      definitionSections = map (
        name:
        builtins.concatStringsSep "\n" [
          "## `${name}`"
          ""
          (renderNode 0 name bundle.definitions.${name})
        ]
      ) definitionNames;
      sections = [
        "# Nixfied Contract Bundle"
        ""
        "Version: `${builtins.toString bundle.version}`"
      ]
      ++ lib.optionals (definitionSections != [ ]) [ "" ]
      ++ lib.intersperse "" definitionSections;
    in
    builtins.concatStringsSep "\n" sections + "\n";
}
