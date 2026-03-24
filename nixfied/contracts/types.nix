{
  lib,
  canonical,
}:
let
  sortNames = attrs: builtins.sort builtins.lessThan (builtins.attrNames attrs);
  nullValue = null;

  isAttrset = value: builtins.typeOf value == "set";
  isList = value: builtins.typeOf value == "list";
  isBool = value: builtins.typeOf value == "bool";
  isInt = value: builtins.typeOf value == "int";
  isString = value: builtins.typeOf value == "string";
  isNumber =
    value:
    let
      valueType = builtins.typeOf value;
    in
    valueType == "int" || valueType == "float";

  nonEmptyString =
    context: value:
    if isString value && value != "" then value else throw "${context}: expected a non-empty string";

  nullableDoc = context: value: if value == null then null else nonEmptyString context value;

  optionalValue = name: value: lib.optionalAttrs (value != null) { "${name}" = value; };

  mapIndexed =
    fn: list: builtins.genList (index: fn index (builtins.elemAt list index)) (builtins.length list);

  literalKind =
    context: value:
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
      throw "${context}: literal values must be string, integer, boolean, or null";

  normalizeDoc = context: raw: nullableDoc "${context}.doc" (raw.doc or null);

  validateConstraintBound =
    context: name: value:
    if value == null then
      null
    else if isInt value then
      value
    else
      throw "${context}.${name}: expected an integer";

  validateNumericBound =
    context: name: value:
    if value == null then
      null
    else if isNumber value then
      value
    else
      throw "${context}.${name}: expected a number";

  ensureOrderedBounds =
    context: lowerName: lowerValue: upperName: upperValue:
    if lowerValue == null || upperValue == null || lowerValue <= upperValue then
      true
    else
      throw "${context}: ${lowerName} must be less than or equal to ${upperName}";

  normalizeField =
    context: fieldName: raw:
    let
      field = if isAttrset raw then raw else throw "${context}.${fieldName}: expected an attrset";
      required =
        if field ? required then
          if isBool field.required then
            field.required
          else
            throw "${context}.${fieldName}.required: expected a bool"
        else
          true;
      schema =
        if field ? schema then
          normalizeNodeImpl "${context}.${fieldName}.schema" field.schema
        else
          throw "${context}.${fieldName}: missing schema";
      doc = nullableDoc "${context}.${fieldName}.doc" (field.doc or null);
    in
    {
      inherit
        required
        schema
        ;
    }
    // optionalValue "doc" doc;

  normalizeRecord =
    context: raw:
    let
      fieldsRaw =
        if raw ? fields then
          if isAttrset raw.fields then raw.fields else throw "${context}.fields: expected an attrset"
        else
          { };
      closed =
        if raw ? closed then
          if isBool raw.closed then raw.closed else throw "${context}.closed: expected a bool"
        else
          true;
      fields = builtins.listToAttrs (
        map (fieldName: {
          name = fieldName;
          value = normalizeField "${context}.fields" fieldName fieldsRaw.${fieldName};
        }) (sortNames fieldsRaw)
      );
    in
    {
      kind = "record";
      inherit
        closed
        fields
        ;
    };

  normalizeTaggedVariant =
    context: tag: closed: variantName: raw:
    let
      variant = normalizeNodeImpl "${context}.variants.${variantName}" (
        raw
        // {
          closed = if raw ? closed then raw.closed else closed;
        }
      );
      tagField =
        variant.fields.${tag} or (throw "${context}.variants.${variantName}: missing tag field '${tag}'");
      tagSchema = tagField.schema;
    in
    if variant.kind != "record" then
      throw "${context}.variants.${variantName}: tagged union variants must be records"
    else if tagSchema.kind != "literal" || tagSchema.value != variantName then
      throw "${context}.variants.${variantName}: tag field '${tag}' must be literal '${variantName}'"
    else
      variant;

  normalizeNodeImpl =
    context: raw:
    let
      node = if isAttrset raw then raw else throw "${context}: expected an attrset";
      kind = nonEmptyString "${context}.kind" (node.kind or (throw "${context}: missing kind"));
      doc = normalizeDoc context node;
      base = {
        inherit kind;
      }
      // optionalValue "doc" doc;
    in
    if kind == "bool" then
      base
    else if kind == "string" then
      let
        minLength = validateConstraintBound context "minLength" (node.minLength or null);
        maxLength = validateConstraintBound context "maxLength" (node.maxLength or null);
        pattern = if node ? pattern then nonEmptyString "${context}.pattern" node.pattern else null;
      in
      assert ensureOrderedBounds context "minLength" minLength "maxLength" maxLength;
      base
      // optionalValue "minLength" minLength
      // optionalValue "maxLength" maxLength
      // optionalValue "pattern" pattern
    else if kind == "integer" then
      let
        minimum = validateConstraintBound context "minimum" (node.minimum or null);
        maximum = validateConstraintBound context "maximum" (node.maximum or null);
      in
      assert ensureOrderedBounds context "minimum" minimum "maximum" maximum;
      base // optionalValue "minimum" minimum // optionalValue "maximum" maximum
    else if kind == "number" then
      let
        minimum = validateNumericBound context "minimum" (node.minimum or null);
        maximum = validateNumericBound context "maximum" (node.maximum or null);
      in
      assert ensureOrderedBounds context "minimum" minimum "maximum" maximum;
      base // optionalValue "minimum" minimum // optionalValue "maximum" maximum
    else if kind == "null" then
      base
    else if kind == "literal" then
      let
        value = node.value or (throw "${context}.value: missing literal value");
      in
      assert (literalKind context value) != "";
      base // { inherit value; }
    else if kind == "enum" then
      let
        values =
          if node ? values then
            if isList node.values && node.values != [ ] then
              node.values
            else
              throw "${context}.values: expected a non-empty list"
          else
            throw "${context}.values: missing enum values";
        firstKind = literalKind "${context}.values[0]" (builtins.elemAt values 0);
        kindsMatch = builtins.all (
          index:
          let
            valueKind = literalKind "${context}.values[${builtins.toString index}]" (
              builtins.elemAt values index
            );
          in
          if valueKind == firstKind then
            true
          else
            throw "${context}.values: enum values must all share the same primitive kind"
        ) (builtins.genList (index: index) (builtins.length values));
      in
      assert kindsMatch;
      base // { inherit values; }
    else if kind == "list" then
      let
        elem = normalizeNodeImpl "${context}.elem" (
          node.elem or (throw "${context}.elem: missing list element schema")
        );
        minItems = validateConstraintBound context "minItems" (node.minItems or null);
        maxItems = validateConstraintBound context "maxItems" (node.maxItems or null);
      in
      assert ensureOrderedBounds context "minItems" minItems "maxItems" maxItems;
      base
      // {
        inherit elem;
      }
      // optionalValue "minItems" minItems
      // optionalValue "maxItems" maxItems
    else if kind == "map" then
      let
        key = normalizeNodeImpl "${context}.key" (node.key or (throw "${context}.key: missing key schema"));
        value = normalizeNodeImpl "${context}.value" (
          node.value or (throw "${context}.value: missing value schema")
        );
      in
      if key.kind != "string" then
        throw "${context}.key: v1 map keys must use kind = \"string\""
      else
        base
        // {
          inherit
            key
            value
            ;
        }
    else if kind == "record" then
      base // normalizeRecord context node
    else if kind == "union" then
      let
        options =
          if node ? options then
            if isList node.options && node.options != [ ] then
              mapIndexed (
                index: option: normalizeNodeImpl "${context}.options[${builtins.toString index}]" option
              ) node.options
            else
              throw "${context}.options: expected a non-empty list"
          else
            throw "${context}.options: missing union options";
      in
      base // { inherit options; }
    else if kind == "taggedUnion" then
      let
        tag = nonEmptyString "${context}.tag" (node.tag or (throw "${context}.tag: missing tag"));
        closed =
          if node ? closed then
            if isBool node.closed then node.closed else throw "${context}.closed: expected a bool"
          else
            true;
        variantsRaw =
          if node ? variants then
            if isAttrset node.variants && node.variants != { } then
              node.variants
            else
              throw "${context}.variants: expected a non-empty attrset"
          else
            throw "${context}.variants: missing variants";
        variants = builtins.listToAttrs (
          map (variantName: {
            name = variantName;
            value = normalizeTaggedVariant context tag closed variantName variantsRaw.${variantName};
          }) (sortNames variantsRaw)
        );
      in
      base
      // {
        kind = "taggedUnion";
        inherit
          tag
          closed
          variants
          ;
      }
    else if kind == "ref" then
      base
      // {
        name = nonEmptyString "${context}.name" (
          node.name or (throw "${context}.name: missing reference name")
        );
      }
    else
      throw "${context}: unsupported kind '${kind}'";

  flattenDefinitions =
    prefix: raw:
    if !isAttrset raw then
      throw "definitions.${prefix}: expected an attrset"
    else if raw ? kind then
      [
        {
          name = prefix;
          value = normalizeNodeImpl "definitions.${prefix}" raw;
        }
      ]
    else
      builtins.concatLists (
        map (name: flattenDefinitions (if prefix == "" then name else "${prefix}.${name}") raw.${name}) (
          sortNames raw
        )
      );

  collectRefs =
    node:
    if node.kind == "ref" then
      [ node.name ]
    else if node.kind == "record" then
      builtins.concatLists (
        map (fieldName: collectRefs node.fields.${fieldName}.schema) (sortNames node.fields)
      )
    else if node.kind == "union" then
      builtins.concatLists (map collectRefs node.options)
    else if node.kind == "taggedUnion" then
      builtins.concatLists (
        map (variantName: collectRefs node.variants.${variantName}) (sortNames node.variants)
      )
    else if node.kind == "list" then
      collectRefs node.elem
    else if node.kind == "map" then
      collectRefs node.key ++ collectRefs node.value
    else
      [ ];

  validateRefs =
    bundle:
    let
      definitionNames = sortNames bundle.definitions;
      defined = builtins.listToAttrs (map (name: lib.nameValuePair name true) definitionNames);
      refsResolve = builtins.all (
        definitionName:
        builtins.all (
          refName:
          if builtins.hasAttr refName defined then
            true
          else
            throw "definitions.${definitionName}: unresolved ref '${refName}'"
        ) (collectRefs bundle.definitions.${definitionName})
      ) definitionNames;
    in
    assert refsResolve;
    bundle;
in
rec {
  bool =
    {
      doc ? nullValue,
    }:
    { kind = "bool"; } // optionalValue "doc" doc;

  string =
    {
      doc ? nullValue,
      minLength ? nullValue,
      maxLength ? nullValue,
      pattern ? nullValue,
    }:
    {
      kind = "string";
    }
    // optionalValue "doc" doc
    // optionalValue "minLength" minLength
    // optionalValue "maxLength" maxLength
    // optionalValue "pattern" pattern;

  integer =
    {
      doc ? nullValue,
      minimum ? nullValue,
      maximum ? nullValue,
    }:
    {
      kind = "integer";
    }
    // optionalValue "doc" doc
    // optionalValue "minimum" minimum
    // optionalValue "maximum" maximum;

  number =
    {
      doc ? nullValue,
      minimum ? nullValue,
      maximum ? nullValue,
    }:
    {
      kind = "number";
    }
    // optionalValue "doc" doc
    // optionalValue "minimum" minimum
    // optionalValue "maximum" maximum;

  null =
    {
      doc ? nullValue,
    }:
    { kind = "null"; } // optionalValue "doc" doc;

  literal =
    {
      value,
      doc ? nullValue,
    }:
    {
      kind = "literal";
      inherit value;
    }
    // optionalValue "doc" doc;

  enum =
    {
      values,
      doc ? nullValue,
    }:
    {
      kind = "enum";
      inherit values;
    }
    // optionalValue "doc" doc;

  list =
    {
      elem,
      doc ? nullValue,
      minItems ? nullValue,
      maxItems ? nullValue,
    }:
    {
      kind = "list";
      inherit elem;
    }
    // optionalValue "doc" doc
    // optionalValue "minItems" minItems
    // optionalValue "maxItems" maxItems;

  map =
    {
      key,
      value,
      doc ? nullValue,
    }:
    {
      kind = "map";
      inherit
        key
        value
        ;
    }
    // optionalValue "doc" doc;

  field =
    {
      schema,
      required ? true,
      doc ? nullValue,
    }:
    {
      inherit
        schema
        required
        ;
    }
    // optionalValue "doc" doc;

  record =
    {
      fields ? { },
      closed ? true,
      doc ? nullValue,
    }:
    {
      kind = "record";
      inherit
        fields
        closed
        ;
    }
    // optionalValue "doc" doc;

  union =
    {
      options,
      doc ? nullValue,
    }:
    {
      kind = "union";
      inherit options;
    }
    // optionalValue "doc" doc;

  taggedUnion =
    {
      tag,
      variants,
      closed ? true,
      doc ? nullValue,
    }:
    {
      kind = "taggedUnion";
      inherit
        tag
        variants
        closed
        ;
    }
    // optionalValue "doc" doc;

  ref =
    {
      name,
      doc ? nullValue,
    }:
    {
      kind = "ref";
      inherit name;
    }
    // optionalValue "doc" doc;

  mkBundle =
    {
      version ? 1,
      definitions,
    }:
    {
      inherit
        version
        definitions
        ;
    };

  toCueIdentifier =
    name:
    let
      sanitized = builtins.replaceStrings [ "." "-" ":" "/" ] [ "__" "_dash_" "_colon_" "_slash_" ] name;
    in
    if builtins.match "[A-Za-z_][A-Za-z0-9_]*" sanitized != null then
      sanitized
    else
      "contract_${sanitized}";

  normalizeBundle =
    raw:
    let
      bundle = if isAttrset raw then raw else throw "contract bundle: expected an attrset";
      version =
        if bundle ? version then
          if isInt bundle.version then
            bundle.version
          else
            throw "contract bundle.version: expected an integer"
        else
          1;
      definitionsRaw =
        if bundle ? definitions then
          if isAttrset bundle.definitions then
            bundle.definitions
          else
            throw "contract bundle.definitions: expected an attrset"
        else
          throw "contract bundle.definitions: missing definitions";
      flattened = flattenDefinitions "" definitionsRaw;
      definitions =
        if flattened != [ ] then
          builtins.listToAttrs flattened
        else
          throw "contract bundle.definitions: expected at least one definition";
      normalized = canonical.canonicalize {
        inherit
          version
          definitions
          ;
      };
    in
    validateRefs normalized;

  definitionNames = bundle: sortNames (normalizeBundle bundle).definitions;

  normalizeNode = normalizeNodeImpl;
}
