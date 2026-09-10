//! Onshape schema presentation for MCP endpoint explanations and schema lookup.
//!
//! The annotation name is Onshape-specific; discriminator discovery belongs to
//! the standard schema catalog. Keep these annotations out of parsed schemas.

use serde_json::Value;

use onshape_openapi::{EndpointDetail, OpenApiSpec, SchemaDetail};

/// Annotate endpoint request and response schemas without mutating the catalog.
pub(super) fn present_endpoint(detail: &mut EndpointDetail, spec: &OpenApiSpec) {
    for schema in [&mut detail.request_body_schema, &mut detail.response_schema]
        .into_iter()
        .flatten()
    {
        *schema = annotate_schema_properties(schema, spec);
    }
}

/// Annotate a component's merged properties without mutating the catalog.
pub(super) fn present_schema(detail: &mut SchemaDetail, spec: &OpenApiSpec) {
    detail.properties = annotate_discriminators(&detail.properties, spec);
}

/// Walk a schema's properties and annotate any `$ref` (or `items.$ref`)
/// that points to a schema with a `discriminator.mapping` by adding an
/// `x-bttype-options` array listing the valid btType values.
///
/// Only examines one level of properties (does not recurse into subtypes).
fn annotate_discriminators(schema: &Value, spec: &OpenApiSpec) -> Value {
    let Some(props) = schema.as_object() else {
        return schema.clone();
    };

    let mut annotated = props.clone();

    for (key, value) in props {
        let annotated_value = annotate_single_property(value, spec);
        if annotated_value != *value {
            annotated.insert(key.clone(), annotated_value);
        }
    }

    Value::Object(annotated)
}

/// Check a single property value for `$ref` or `items.$ref` pointing to
/// a schema with a discriminator, and annotate it with `x-bttype-options`.
fn annotate_single_property(value: &Value, spec: &OpenApiSpec) -> Value {
    // Direct $ref
    if let Some(ref_str) = value.get("$ref").and_then(Value::as_str)
        && let Some(options) = spec.discriminator_options(ref_str)
    {
        let mut annotated = value.as_object().cloned().unwrap_or_default();
        annotated.insert("x-bttype-options".to_string(), Value::from(options));
        return Value::Object(annotated);
    }

    // items.$ref (for array properties)
    if let Some(items) = value.get("items")
        && let Some(ref_str) = items.get("$ref").and_then(Value::as_str)
        && let Some(options) = spec.discriminator_options(ref_str)
    {
        let mut annotated_items = items.as_object().cloned().unwrap_or_default();
        annotated_items.insert("x-bttype-options".to_string(), Value::from(options));
        let mut annotated = value.as_object().cloned().unwrap_or_default();
        annotated.insert("items".to_string(), Value::Object(annotated_items));
        return Value::Object(annotated);
    }

    value.clone()
}

/// Annotate a resolved schema's `properties` with discriminator info.
///
/// If the schema has a `properties` object, walk it and annotate any `$ref`
/// properties that point to discriminator schemas. Returns the schema with
/// the annotated properties in place.
fn annotate_schema_properties(schema: &Value, spec: &OpenApiSpec) -> Value {
    let mut result = schema.clone();
    let Some(obj) = result.as_object_mut() else {
        return result;
    };

    if let Some(props) = obj.get("properties").cloned() {
        obj.insert(
            "properties".to_string(),
            annotate_discriminators(&props, spec),
        );
    }

    if let Some(all_of) = obj.get_mut("allOf").and_then(Value::as_array_mut) {
        for item in all_of {
            let Some(props) = item.get("properties").cloned() else {
                continue;
            };
            if let Some(item_obj) = item.as_object_mut() {
                item_obj.insert(
                    "properties".to_string(),
                    annotate_discriminators(&props, spec),
                );
            }
        }
    }

    result
}
