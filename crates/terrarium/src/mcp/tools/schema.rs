//! Deriving a tool's `inputSchema` from its input struct.

use schemars::JsonSchema;
use serde_json::{Value, json};

/// Derives a tool's `inputSchema` from its input struct.
///
/// Normalized to the shape MCP clients were served before schemas were derived,
/// so switching to derivation did not change the advertised contract:
///
/// - `$schema` and `title` are dropped — neither means anything to a client, and
///   `title` would leak the Rust type name into the tool surface.
/// - An `Option<T>` field advertises `T`'s own type rather than
///   `[T, "null"]`. Optionality is already carried by absence from `required`,
///   and the union form invites clients to send an explicit `null`.
/// - `"default": null` is dropped; it states only that an absent field is absent.
pub(crate) fn schema_for<T: JsonSchema>() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| json!({ "type": "object" }));
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    normalize_schema(&mut schema);
    schema
}

/// Strips the two schemars artifacts that have no meaning in an MCP tool schema:
/// the `null` arm of an optional field's type union, and a `null` default.
fn normalize_schema(node: &mut Value) {
    match node {
        Value::Object(obj) => {
            if obj.get("default") == Some(&Value::Null) {
                obj.remove("default");
            }
            if let Some(Value::Array(types)) = obj.get("type") {
                let named: Vec<Value> = types
                    .iter()
                    .filter(|t| t.as_str() != Some("null"))
                    .cloned()
                    .collect();
                // Collapse only the ordinary `Option<T>` case: one real type
                // beside `null`. A genuine multi-type union is left alone.
                if named.len() == 1 && types.len() == 2 {
                    obj.insert("type".to_string(), named[0].clone());
                }
            }
            for value in obj.values_mut() {
                normalize_schema(value);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_schema),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::common::DockerRunInput;
    use crate::mcp::tools::inputs::{GitLogInput, GitShowInput, NoInput};

    /// Pins the exact `inputSchema` shape MCP clients are served. Schemas are
    /// derived from the input structs, so a schemars upgrade could silently
    /// change the advertised contract — this fails instead.
    #[test]
    fn derived_schema_matches_advertised_contract() {
        assert_eq!(
            schema_for::<NoInput>(),
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional working directory relative to project root"
                    },
                    "project": {
                        "type": "string",
                        "description": "Project name to target when multiple projects are available"
                    }
                },
                "additionalProperties": false
            }),
            "optional fields must advertise a plain type, never a [T, null] union"
        );

        let show = schema_for::<GitShowInput>();
        assert_eq!(
            show.get("required").and_then(Value::as_array),
            Some(&vec![json!("revision")]),
            "a non-Option field must land in `required`"
        );
        assert_eq!(show["properties"]["revision"]["type"], json!("string"));
        assert!(
            show["properties"]["patch"].get("default").is_none(),
            "`default: null` must not leak into the tool surface"
        );
    }

    /// Numeric bounds were carried by hand-written schemas and are now
    /// `#[schemars(range(...))]` — verify they still reach the client.
    #[test]
    fn derived_schema_keeps_numeric_bounds() {
        let log = schema_for::<GitLogInput>();
        assert_eq!(log["properties"]["limit"]["minimum"], json!(1));
        assert_eq!(log["properties"]["limit"]["maximum"], json!(50));
    }

    /// `Vec<String>` fields must still advertise as a typed array.
    #[test]
    fn derived_schema_types_string_arrays() {
        let docker = schema_for::<DockerRunInput>();
        assert_eq!(docker["properties"]["args"]["type"], json!("array"));
        assert_eq!(
            docker["properties"]["args"]["items"]["type"],
            json!("string")
        );
        assert_eq!(
            docker.get("required").and_then(Value::as_array),
            Some(&vec![json!("args")])
        );
    }
}
