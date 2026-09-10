//! Host-configured presentation and name resolution for the four API tools.

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::Value;

use super::{ApiCallInput, ApiExplainInput, ApiSchemaInput, ApiSearchInput};

/// The operation behind a configured tool name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// Search the specification for endpoints.
    Search,
    /// Explain one endpoint.
    Explain,
    /// Prepare an API request for execution.
    Call,
    /// Look up a component schema.
    Schema,
}

/// Host wording for one tool, separate from its input structure and behavior.
#[derive(Clone, Copy)]
pub struct ToolDefinition<'a> {
    /// The operation this name invokes.
    pub kind: ToolKind,
    /// The advertised and accepted tool name.
    pub name: &'a str,
    /// The advertised tool description, including any cross-tool guidance.
    pub description: &'a str,
    /// Optional replacement for the input schema's top-level description.
    pub input_description: Option<&'a str>,
    /// Description overrides for named top-level input properties.
    pub field_descriptions: &'a [(&'a str, &'a str)],
}

/// Validated tool definitions used for both discovery and dispatch.
#[derive(Debug)]
pub struct ToolSet {
    entries: Vec<(ToolKind, Tool)>,
}

impl ToolSet {
    /// Build one definition for each API operation in the supplied display order.
    ///
    /// # Errors
    ///
    /// Rejects blank or duplicate names, duplicate operations, and description
    /// overrides that do not name an object-valued input property schema.
    pub fn new(definitions: &[ToolDefinition<'_>; 4]) -> Result<Self, String> {
        let mut names = HashSet::new();
        let mut kinds = HashSet::new();
        let mut entries = Vec::with_capacity(definitions.len());
        for definition in definitions {
            if definition.name.trim().is_empty() {
                return Err("API tool name must not be blank".into());
            }
            if !names.insert(definition.name) {
                return Err(format!("duplicate API tool name: {}", definition.name));
            }
            if !kinds.insert(definition.kind) {
                return Err(format!("duplicate API tool kind: {:?}", definition.kind));
            }
            entries.push((definition.kind, definition.build()?));
        }
        Ok(Self { entries })
    }

    /// Return the advertised tools in their configured order.
    pub fn list(&self) -> Vec<Tool> {
        self.entries.iter().map(|(_, tool)| tool.clone()).collect()
    }

    /// Resolve an advertised name to its operation, leaving other names to the host.
    pub fn resolve(&self, name: &str) -> Option<ToolKind> {
        self.entries
            .iter()
            .find_map(|(kind, tool)| (tool.name == name).then_some(*kind))
    }
}

impl ToolDefinition<'_> {
    /// Apply documentation overrides while retaining the generated input structure.
    #[allow(clippy::expect_used)] // These concrete input types always produce object schemas.
    fn build(self) -> Result<Tool, String> {
        let schema = match self.kind {
            ToolKind::Search => schemars::schema_for!(ApiSearchInput),
            ToolKind::Explain => schemars::schema_for!(ApiExplainInput),
            ToolKind::Call => schemars::schema_for!(ApiCallInput),
            ToolKind::Schema => schemars::schema_for!(ApiSchemaInput),
        };
        let mut input_schema = serde_json::to_value(schema)
            .expect("API input schema serialization should never fail")
            .as_object()
            .cloned()
            .expect("API input schema should be an object");
        if let Some(description) = self.input_description {
            input_schema.insert("description".into(), Value::String(description.into()));
        }
        let properties = input_schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .expect("API input schema should define properties");
        for &(name, description) in self.field_descriptions {
            let property = properties
                .get_mut(name)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    format!(
                        "invalid API input description field {name:?} for {:?}",
                        self.kind
                    )
                })?;
            property.insert("description".into(), Value::String(description.into()));
        }
        let invokes_api = self.kind == ToolKind::Call;
        Ok(Tool::new(
            self.name.to_owned(),
            self.description.to_owned(),
            Arc::new(input_schema),
        )
        .annotate(
            ToolAnnotations::new()
                .read_only(!invokes_api)
                .destructive(invokes_api),
        ))
    }
}
