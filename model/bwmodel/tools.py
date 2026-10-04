"""Tool definitions in the OpenAI `tools` format, and argument checking.

The host passes tools to llama-server, which renders them with the model's own
chat template, so the training data stores them in that same format.
"""

import json
from pathlib import Path

JSON_TYPES = {
    "string": str,
    "integer": int,
    "number": (int, float),
    "boolean": bool,
    "array": list,
    "object": dict,
}


def load_tools(path):
    tools = json.loads(Path(path).read_text(encoding="utf-8"))
    for tool in tools:
        check_tool_definition(tool)
    return tools


def tool_name(tool):
    return tool["function"]["name"]


def check_tool_definition(tool):
    if tool.get("type") != "function":
        raise ValueError(f"tool must have type 'function': {tool}")
    fn = tool.get("function") or {}
    if not fn.get("name") or not fn.get("description"):
        raise ValueError(f"tool needs a name and description: {tool}")
    params = fn.get("parameters") or {}
    if params.get("type") != "object":
        raise ValueError(f"tool `{fn['name']}` parameters must be an object schema")
    for key in params.get("required", []):
        if key not in params.get("properties", {}):
            raise ValueError(f"tool `{fn['name']}` requires unknown property `{key}`")


def validate(value, schema, path="arguments"):
    """Returns a list of problems with `value` against a small JSON Schema subset
    (type, properties, required, enum, items). Empty means valid."""
    problems = []
    kind = schema.get("type")
    if kind:
        expected = JSON_TYPES.get(kind)
        bad_bool = kind in ("integer", "number") and isinstance(value, bool)
        if expected and (not isinstance(value, expected) or bad_bool):
            return [f"{path} should be {kind}, got {type(value).__name__}"]
    if "enum" in schema and value not in schema["enum"]:
        problems.append(f"{path} must be one of {schema['enum']}, got {value!r}")
    if isinstance(value, dict):
        props = schema.get("properties", {})
        for key in schema.get("required", []):
            if key not in value:
                problems.append(f"{path}.{key} is required")
        for key, item in value.items():
            if key in props:
                problems += validate(item, props[key], f"{path}.{key}")
            elif schema.get("additionalProperties") is False or props:
                problems.append(f"{path}.{key} is not a parameter")
    if isinstance(value, list) and "items" in schema:
        for i, item in enumerate(value):
            problems += validate(item, schema["items"], f"{path}[{i}]")
    return problems


def check_call(call, tools):
    """Checks one tool call (`{"name", "arguments"}` with arguments as a dict or
    JSON string) against the offered tools. Returns (arguments, problems)."""
    by_name = {tool_name(t): t for t in tools}
    name = call.get("name")
    if name not in by_name:
        return None, [f"unknown tool `{name}`"]
    args = call.get("arguments")
    if isinstance(args, str):
        try:
            args = json.loads(args) if args.strip() else {}
        except json.JSONDecodeError as e:
            return None, [f"arguments are not valid JSON: {e}"]
    if not isinstance(args, dict):
        return None, ["arguments must be a JSON object"]
    return args, validate(args, by_name[name]["function"]["parameters"])
