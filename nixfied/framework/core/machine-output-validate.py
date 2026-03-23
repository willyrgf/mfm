#!/usr/bin/env python3
import json
import sys


def fail(message: str) -> int:
    print(message, file=sys.stderr)
    return 1


def format_path(path: str, segment: str) -> str:
    if path == "$":
        if segment.startswith("["):
            return f"${segment}"
        return f"$.{segment}"
    if segment.startswith("["):
        return f"{path}{segment}"
    return f"{path}.{segment}"


def type_ok(expected: str, value) -> bool:
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "null":
        return value is None
    return False


def validate(schema, value, path="$"):
    errors = []
    if not isinstance(schema, dict):
        return [f"{path}: schema must be an object"]

    expected_type = schema.get("type")
    if expected_type is not None:
        if not isinstance(expected_type, str):
            return [f"{path}: schema type must be a string"]
        if not type_ok(expected_type, value):
            return [f"{path}: expected {expected_type}"]

    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{path}: value is not in enum")

    if isinstance(value, dict):
        properties = schema.get("properties", {})
        if properties is not None and not isinstance(properties, dict):
            errors.append(f"{path}: properties must be an object")
        else:
            required = schema.get("required", [])
            if not isinstance(required, list):
                errors.append(f"{path}: required must be a list")
            else:
                for key in required:
                    if key not in value:
                        errors.append(f"{format_path(path, key)}: missing required property")
            for key, subschema in properties.items():
                if key in value:
                    errors.extend(validate(subschema, value[key], format_path(path, key)))
            if schema.get("additionalProperties", True) is False:
                for key in value:
                    if key not in properties:
                        errors.append(f"{format_path(path, key)}: additional properties are not allowed")

    if isinstance(value, list):
        items = schema.get("items")
        if items is not None:
            for index, item in enumerate(value):
                errors.extend(validate(items, item, format_path(path, f"[{index}]")))
        min_items = schema.get("minItems")
        max_items = schema.get("maxItems")
        if min_items is not None and len(value) < min_items:
            errors.append(f"{path}: expected at least {min_items} items")
        if max_items is not None and len(value) > max_items:
            errors.append(f"{path}: expected at most {max_items} items")

    if isinstance(value, str):
        min_length = schema.get("minLength")
        max_length = schema.get("maxLength")
        if min_length is not None and len(value) < min_length:
            errors.append(f"{path}: expected minimum length {min_length}")
        if max_length is not None and len(value) > max_length:
            errors.append(f"{path}: expected maximum length {max_length}")

    return errors


def main(argv) -> int:
    if len(argv) != 3:
        return fail("usage: machine-output-validate.py <schema-or-empty> <json-file>")

    schema_path = argv[1]
    value_path = argv[2]

    try:
        with open(value_path, "r", encoding="utf-8") as handle:
            value = json.load(handle)
    except json.JSONDecodeError as exc:
        return fail(f"$: invalid json: {exc.msg}")

    if schema_path == "":
        return 0

    try:
        with open(schema_path, "r", encoding="utf-8") as handle:
            schema = json.load(handle)
    except json.JSONDecodeError as exc:
        return fail(f"$: invalid schema json: {exc.msg}")

    errors = validate(schema, value)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
