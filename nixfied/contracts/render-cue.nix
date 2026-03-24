{
  lib,
  types,
}:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);

  indent = level: builtins.concatStringsSep "" (builtins.genList (_: "  ") level);

  renderScalar =
    value:
    let
      valueType = builtins.typeOf value;
    in
    if valueType == "string" then
      builtins.toJSON value
    else if valueType == "int" || valueType == "float" then
      builtins.toString value
    else if valueType == "bool" then
      if value then "true" else "false"
    else if valueType == "null" then
      "null"
    else
      throw "render-cue: unsupported scalar type '${valueType}'";

  renderFieldLabel =
    name: if builtins.match "[A-Za-z_][A-Za-z0-9_]*" name != null then name else builtins.toJSON name;

  needsParens =
    expr:
    lib.hasInfix " " expr || lib.hasInfix "\n" expr || lib.hasInfix "&" expr || lib.hasInfix "|" expr;

  wrapExpr = expr: if needsParens expr then "(${expr})" else expr;

  stringConstraints =
    node:
    lib.optionals (node ? minLength) [ "strings.MinRunes(${builtins.toString node.minLength})" ]
    ++ lib.optionals (node ? maxLength) [ "strings.MaxRunes(${builtins.toString node.maxLength})" ]
    ++ lib.optionals (node ? pattern) [ "=~${builtins.toJSON node.pattern}" ];

  numericConstraints =
    node:
    lib.optionals (node ? minimum) [ ">=${builtins.toString node.minimum}" ]
    ++ lib.optionals (node ? maximum) [ "<=${builtins.toString node.maximum}" ];

  renderRecordBody =
    level: node:
    let
      fieldLines = map (
        fieldName:
        let
          field = node.fields.${fieldName};
          separator = if field.required then ":" else "?:";
        in
        "${indent (level + 1)}${renderFieldLabel fieldName}${separator} ${
          renderNode (level + 1) field.schema
        }"
      ) (sortNames node.fields);
      body =
        if fieldLines == [ ] then
          "{\n${indent level}}"
        else
          "{\n${builtins.concatStringsSep "\n" fieldLines}\n${indent level}}";
    in
    if node.closed then "close(${body})" else body;

  renderMap =
    level: node:
    let
      keyExpr = if node.key ? pattern then "=~${builtins.toJSON node.key.pattern}" else "string";
    in
    "{ [${keyExpr}]: ${renderNode level node.value} }";

  renderNode =
    level: node:
    if node.kind == "bool" then
      "bool"
    else if node.kind == "string" then
      builtins.concatStringsSep " & " ([ "string" ] ++ stringConstraints node)
    else if node.kind == "integer" then
      builtins.concatStringsSep " & " ([ "int" ] ++ numericConstraints node)
    else if node.kind == "number" then
      builtins.concatStringsSep " & " ([ "number" ] ++ numericConstraints node)
    else if node.kind == "null" then
      "null"
    else if node.kind == "literal" then
      renderScalar node.value
    else if node.kind == "enum" then
      builtins.concatStringsSep " | " (map renderScalar node.values)
    else if node.kind == "list" then
      let
        elemExpr = wrapExpr (renderNode level node.elem);
        base = "[...${elemExpr}]";
        constraints =
          lib.optionals (node ? minItems) [ "list.MinItems(${builtins.toString node.minItems})" ]
          ++ lib.optionals (node ? maxItems) [ "list.MaxItems(${builtins.toString node.maxItems})" ];
      in
      builtins.concatStringsSep " & " ([ base ] ++ constraints)
    else if node.kind == "map" then
      renderMap level node
    else if node.kind == "record" then
      renderRecordBody level node
    else if node.kind == "union" then
      builtins.concatStringsSep " | " (map (option: wrapExpr (renderNode level option)) node.options)
    else if node.kind == "taggedUnion" then
      builtins.concatStringsSep " | " (
        map (variantName: wrapExpr (renderRecordBody level node.variants.${variantName})) (
          sortNames node.variants
        )
      )
    else if node.kind == "ref" then
      "#${types.toCueIdentifier node.name}"
    else
      throw "render-cue: unsupported node kind '${node.kind}'";

  collectImports =
    node:
    let
      direct =
        lib.optionals (node.kind == "string" && ((node ? minLength) || (node ? maxLength))) [ "strings" ]
        ++ lib.optionals (node.kind == "list" && ((node ? minItems) || (node ? maxItems))) [ "list" ];
      nested =
        if node.kind == "record" then
          builtins.concatLists (
            map (fieldName: collectImports node.fields.${fieldName}.schema) (sortNames node.fields)
          )
        else if node.kind == "union" then
          builtins.concatLists (map collectImports node.options)
        else if node.kind == "taggedUnion" then
          builtins.concatLists (
            map (variantName: collectImports node.variants.${variantName}) (sortNames node.variants)
          )
        else if node.kind == "list" then
          collectImports node.elem
        else if node.kind == "map" then
          collectImports node.key ++ collectImports node.value
        else
          [ ];
    in
    direct ++ nested;

  renderDefinition = name: node: "#${types.toCueIdentifier name}: ${renderNode 0 node}";
in
{
  definition =
    {
      bundle,
      name,
    }:
    let
      normalized = types.normalizeBundle bundle;
      node =
        normalized.definitions.${name} or (throw "render-cue.definition: unknown definition '${name}'");
      imports = lib.unique (builtins.sort builtins.lessThan (collectImports node));
      sections = [
        "package nixfied"
      ]
      ++ lib.optionals (imports != [ ]) ([ "" ] ++ map (importName: "import \"${importName}\"") imports)
      ++ [
        ""
        (renderDefinition name node)
      ];
    in
    builtins.concatStringsSep "\n" sections + "\n";

  bundle =
    rawBundle:
    let
      bundle = types.normalizeBundle rawBundle;
      definitionNames = sortNames bundle.definitions;
      imports = lib.unique (
        builtins.sort builtins.lessThan (
          builtins.concatLists (map (name: collectImports bundle.definitions.${name}) definitionNames)
        )
      );
      definitionLines = map (name: renderDefinition name bundle.definitions.${name}) definitionNames;
      sections = [
        "package nixfied"
      ]
      ++ lib.optionals (imports != [ ]) ([ "" ] ++ map (importName: "import \"${importName}\"") imports)
      ++ lib.optionals (definitionLines != [ ]) [ "" ]
      ++ definitionLines;
    in
    builtins.concatStringsSep "\n" sections + "\n";
}
