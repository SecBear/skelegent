//! JSON Schema transformation utilities for LLM compatibility.
//!
//! LLMs support different JSON Schema subsets. OpenAPI schemas often include
//! keywords — `format`, `pattern`, `minLength`, etc. — that models silently
//! ignore or actively choke on. These utilities strip the noise so only
//! semantically meaningful keywords reach the model.

use serde_json::{Map, Value};

/// Keywords removed at every level of the schema tree.
///
/// These are either pure metadata (`$schema`, `$id`), validation constraints
/// the model cannot enforce (`format`, `pattern`, range/length bounds), or
/// decoration that adds noise without improving model behavior (`examples`,
/// `default`).
const STRIPPED_KEYWORDS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "$defs",
    "format",
    "examples",
    "default",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "pattern",
];

/// Transform a JSON Schema for LLM consumption by stripping unsupported keywords.
///
/// Removes:
/// - Metadata: `$schema`, `$id`, `$comment`, `$defs`
/// - Validation constraints: `format`, `pattern`, `minItems`, `maxItems`,
///   `minLength`, `maxLength`, `minimum`, `maximum`
/// - Noise: `examples`, `default`
/// - `additionalProperties` when `true` (the implied default — conveys nothing)
///
/// Preserves: `type`, `properties`, `required`, `items`, `enum`, `const`,
/// `description`, `anyOf`, `oneOf`, `allOf`, and any keyword not in the
/// strip list.
///
/// The transformation is **recursive**: nested schemas inside `properties`,
/// `items`, `anyOf`/`oneOf`/`allOf`, etc. are cleaned in the same pass.
pub fn strip_unsupported(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, val) in map {
                // Drop statically listed noisy keywords.
                if STRIPPED_KEYWORDS.contains(&key.as_str()) {
                    continue;
                }
                // `additionalProperties: true` is the implicit default; drop it.
                if key == "additionalProperties" && val == &Value::Bool(true) {
                    continue;
                }
                // Recurse so nested schemas are cleaned in the same pass.
                out.insert(key.clone(), strip_unsupported(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(strip_unsupported).collect()),
        // Scalars (strings, numbers, booleans, null) pass through unchanged.
        other => other.clone(),
    }
}

/// Simplify `oneOf`/`anyOf` with a single variant to just that variant.
///
/// `{"oneOf": [{"type": "string"}]}` becomes `{"type": "string"}`.
///
/// When sibling keys exist alongside the single-variant union (e.g. a
/// `description` at the outer level), they are merged into the result:
/// the variant's keys take priority, and sibling keys the variant does
/// not define fill in. This preserves `description` fields that OpenAPI
/// schemas place adjacent to the union.
///
/// The transformation is applied **recursively** to all nested schemas,
/// including the elements of multi-variant unions and `allOf` arrays.
pub fn simplify_unions(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            // Holds the collapsed single-variant, if we find one.
            let mut replacement: Option<Value> = None;

            for (key, val) in map {
                let is_collapsible_union = matches!(key.as_str(), "oneOf" | "anyOf");
                if is_collapsible_union
                    && let Value::Array(variants) = val
                    && variants.len() == 1
                {
                    // Recurse into the single variant before collapsing.
                    replacement = Some(simplify_unions(&variants[0]));
                    continue; // omit the union key from the output
                }
                out.insert(key.clone(), simplify_unions(val));
            }

            match replacement {
                Some(Value::Object(mut rep_map)) => {
                    // Sibling keys (e.g. `description`) fill in what the variant
                    // doesn't define. Variant keys take priority on conflict.
                    for (k, v) in out {
                        rep_map.entry(k).or_insert(v);
                    }
                    Value::Object(rep_map)
                }
                Some(other) => {
                    // Variant collapsed to a non-object (edge case: `$ref` string,
                    // boolean schema). Return it directly if there are no siblings;
                    // otherwise preserve the sibling object and discard the degenerate variant.
                    if out.is_empty() {
                        other
                    } else {
                        Value::Object(out)
                    }
                }
                None => Value::Object(out),
            }
        }
        Value::Array(arr) => Value::Array(arr.iter().map(simplify_unions).collect()),
        other => other.clone(),
    }
}

/// Full pipeline: strip unsupported keywords, then simplify single-variant unions.
///
/// Equivalent to `simplify_unions(&strip_unsupported(schema))`. Use this as
/// the primary entry point when preparing a tool's `input_schema` for
/// serialization into an LLM API request.
pub fn transform_for_llm(schema: &Value) -> Value {
    simplify_unions(&strip_unsupported(schema))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_format_keyword() {
        let input = json!({
            "type": "string",
            "format": "date-time",
            "description": "An ISO-8601 timestamp"
        });
        let out = strip_unsupported(&input);
        assert!(out.get("format").is_none(), "format should be stripped");
        assert_eq!(out["type"], "string");
        assert_eq!(out["description"], "An ISO-8601 timestamp");
    }

    #[test]
    fn preserves_description() {
        let input = json!({
            "type": "object",
            "description": "A user record",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The user's name"
                }
            }
        });
        let out = strip_unsupported(&input);
        assert_eq!(out["description"], "A user record");
        assert_eq!(out["properties"]["name"]["description"], "The user's name");
    }

    #[test]
    fn simplifies_single_variant_oneof() {
        let input = json!({"oneOf": [{"type": "string"}]});
        let out = simplify_unions(&input);
        assert_eq!(out, json!({"type": "string"}));
    }

    #[test]
    fn handles_nested_objects() {
        let input = json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "object",
                    "format": "address",
                    "properties": {
                        "zip": {
                            "type": "string",
                            "pattern": "\\d{5}",
                            "minLength": 5,
                            "maxLength": 5,
                            "description": "US ZIP code"
                        }
                    }
                }
            }
        });
        let out = strip_unsupported(&input);
        let zip = &out["properties"]["address"]["properties"]["zip"];
        assert!(zip.get("pattern").is_none(), "nested pattern stripped");
        assert!(zip.get("minLength").is_none(), "nested minLength stripped");
        assert!(zip.get("maxLength").is_none(), "nested maxLength stripped");
        assert_eq!(zip["type"], "string");
        assert_eq!(zip["description"], "US ZIP code", "description preserved");
        // format on the address object itself is also stripped
        assert!(
            out["properties"]["address"].get("format").is_none(),
            "format on nested object stripped"
        );
    }

    #[test]
    fn preserves_enum() {
        let input = json!({
            "type": "string",
            "enum": ["red", "green", "blue"],
            "format": "color",
            "default": "red"
        });
        let out = strip_unsupported(&input);
        assert_eq!(out["enum"], json!(["red", "green", "blue"]));
        assert!(out.get("format").is_none(), "format stripped");
        assert!(out.get("default").is_none(), "default stripped");
    }
}
