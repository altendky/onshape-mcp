//! Standard component schema inspection, independent of Onshape presentation.
//!
//! References are deliberately resolved one level at a time. Schema lookup
//! preserves the existing one-parent-level inheritance behavior.

use std::collections::HashMap;

use serde_json::Value;

use crate::{OpenApiError, SchemaDetail};

#[derive(Debug)]
pub struct SchemaCatalog {
    components: HashMap<String, Value>,
}

impl SchemaCatalog {
    pub fn from_root(root: &Value) -> Self {
        let mut components = HashMap::new();
        if let Some(schemas) = root
            .pointer("/components/schemas")
            .and_then(Value::as_object)
        {
            for (name, schema) in schemas {
                components.insert(name.clone(), schema.clone());
            }
        }
        Self { components }
    }

    /// Look up a component schema by name and return its detail.
    ///
    /// Merges parent properties (from `allOf.$ref`) into a flat `properties`
    /// object and includes standard discriminator metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the schema name is not found in the spec's components.
    pub fn lookup(&self, name: &str) -> Result<SchemaDetail, OpenApiError> {
        let schema = self
            .components
            .get(name)
            .ok_or_else(|| OpenApiError::SchemaNotFound {
                schema_name: name.to_string(),
            })?;

        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .map(String::from);

        // Extract discriminator info from this schema.
        let discriminator = schema.get("discriminator");
        let discriminator_property = discriminator
            .and_then(|d| d.get("propertyName"))
            .and_then(Value::as_str)
            .map(String::from);
        let subtypes = discriminator
            .and_then(|d| d.get("mapping"))
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect::<Vec<_>>());

        // Merge properties from allOf (parent) and own properties.
        let mut merged_props = serde_json::Map::new();
        let mut parent = None;
        let mut required: Vec<String> = Vec::new();

        // Collect required from the top-level schema.
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(s) = r.as_str() {
                    required.push(s.to_string());
                }
            }
        }

        // Walk allOf to find parent ref and merge properties.
        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for item in all_of {
                if let Some(ref_str) = item.get("$ref").and_then(Value::as_str) {
                    // This is the parent reference.
                    if let Some(parent_name) = ref_str.strip_prefix("#/components/schemas/") {
                        parent = Some(parent_name.to_string());
                        // Merge parent properties (one level only — transitive
                        // ancestry is accessible via the `parent` field).
                        if let Some(parent_schema) = self.components.get(parent_name) {
                            Self::merge_props_and_required(
                                parent_schema,
                                &mut merged_props,
                                &mut required,
                            );
                            // Also merge properties/required from the parent's
                            // own allOf inline blocks (non-$ref items).
                            if let Some(parent_all_of) =
                                parent_schema.get("allOf").and_then(Value::as_array)
                            {
                                for parent_item in parent_all_of {
                                    if parent_item.get("$ref").is_some() {
                                        continue;
                                    }
                                    Self::merge_props_and_required(
                                        parent_item,
                                        &mut merged_props,
                                        &mut required,
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Inline properties/required from allOf item.
                    Self::merge_props_and_required(item, &mut merged_props, &mut required);
                }
            }
        }

        // Merge top-level properties (override parent if same key).
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, v) in props {
                merged_props.insert(k.clone(), v.clone());
            }
        }

        Ok(SchemaDetail {
            name: name.to_string(),
            description,
            parent,
            properties: Value::Object(merged_props),
            required,
            subtypes,
            discriminator_property,
        })
    }

    /// Merge `properties` and `required` from `source` into the accumulators.
    ///
    /// Used during `lookup` to fold parent (and parent-allOf-inline)
    /// properties into the child's merged view.
    fn merge_props_and_required(
        source: &Value,
        merged_props: &mut serde_json::Map<String, Value>,
        required: &mut Vec<String>,
    ) {
        if let Some(props) = source.get("properties").and_then(Value::as_object) {
            for (k, v) in props {
                merged_props.insert(k.clone(), v.clone());
            }
        }
        if let Some(req) = source.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(s) = r.as_str()
                    && !required.contains(&s.to_string())
                {
                    required.push(s.to_string());
                }
            }
        }
    }

    /// Given a `$ref` string, check if the referenced schema has a discriminator
    /// mapping. If so, return the mapping keys as a `Vec<Value>` of strings.
    pub fn discriminator_options(&self, ref_str: &str) -> Option<Vec<Value>> {
        let name = ref_str.strip_prefix("#/components/schemas/")?;
        let schema = self.components.get(name)?;
        let mapping = schema.get("discriminator")?.get("mapping")?.as_object()?;
        let options: Vec<Value> = mapping.keys().cloned().map(Value::String).collect();
        if options.is_empty() {
            None
        } else {
            Some(options)
        }
    }

    /// Resolve a single level of `$ref` — replaces the `$ref` pointer with the
    /// referenced schema. Does NOT recursively resolve nested `$ref`s (to avoid
    /// unbounded expansion of the spec).
    pub fn resolve_ref_shallow(&self, schema: &Value) -> Value {
        if let Some(ref_str) = schema.get("$ref").and_then(Value::as_str)
            && let Some(name) = ref_str.strip_prefix("#/components/schemas/")
            && let Some(resolved) = self.components.get(name)
        {
            return resolved.clone();
        }
        schema.clone()
    }
}
