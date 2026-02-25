//! MCP resource definitions for Onshape integration.
//!
//! This crate provides MCP resources generated at compile time from markdown
//! documentation in `docs/src/mcp-resources/`. Each subdirectory with an
//! `index.md` becomes a resource group, and each entry in the index becomes
//! an MCP resource.
//!
//! ## Adding resources
//!
//! To add a new resource to an existing group (e.g. `insights`):
//! 1. Create `docs/src/mcp-resources/insights/new-topic.md`
//! 2. Add an entry to `docs/src/mcp-resources/insights/index.md`:
//!    `- [New Topic](new-topic.md) — Description of the new topic`
//!
//! To add a new resource group:
//! 1. Create `docs/src/mcp-resources/new-group/index.md`
//! 2. Add entries following the same format
//!
//! No Rust code changes are needed in either case.

use rmcp::model::{
    Annotated, Annotations, RawResource, ReadResourceResult, ResourceContents, Role,
};

// ============================================================================
// Resource Entry (compile-time data structure)
// ============================================================================

/// A single resource entry, populated at compile time by the build script.
pub struct ResourceEntry {
    /// The resource group (e.g. `"insights"`). Used as the URI scheme.
    pub group: &'static str,
    /// The resource name derived from the filename stem (e.g. `"shaded-views"`).
    pub name: &'static str,
    /// Human-readable title (e.g. `"Shaded Views"`).
    pub title: &'static str,
    /// Brief description for resource listings.
    pub description: &'static str,
    /// The full URI (e.g. `"insights:shaded-views"`).
    pub uri: &'static str,
    /// The full markdown content of the resource.
    pub content: &'static str,
}

// Include the generated resource catalog.
// Generated code uses raw strings with uniform hashing for simplicity.
#[allow(clippy::needless_raw_string_hashes)]
mod generated {
    use super::ResourceEntry;
    include!(concat!(env!("OUT_DIR"), "/resources_generated.rs"));
}
pub use generated::RESOURCES;

// ============================================================================
// Effect Type
// ============================================================================

/// Result of dispatching a resource operation.
///
/// Mirrors the [`ToolResult`](../onshape_mcp_core/tools/enum.ToolResult.html)
/// pattern. Currently all resources are static, so only `Immediate` is used.
/// The enum exists to establish a common effects-as-data pattern that can be
/// extended if resources ever need I/O.
pub enum ResourceResult {
    /// Resource operation completed immediately with no I/O needed.
    Immediate(Result<ReadResourceResult, ResourceError>),
}

/// Errors that can occur when reading a resource.
#[derive(Debug)]
pub enum ResourceError {
    /// The requested URI does not match any known resource.
    NotFound(String),
}

// ============================================================================
// Public API
// ============================================================================

/// List all available MCP resources.
///
/// Returns resource metadata (URI, name, title, description) with annotations
/// marking them as intended for the assistant (LLM) with moderately high
/// priority.
#[must_use]
pub fn list_resources() -> Vec<Annotated<RawResource>> {
    RESOURCES
        .iter()
        .map(|entry| Annotated {
            raw: RawResource {
                uri: entry.uri.into(),
                name: entry.name.into(),
                title: Some(entry.title.into()),
                description: Some(entry.description.into()),
                mime_type: Some("text/markdown".into()),
                size: Some(u32::try_from(entry.content.len()).unwrap_or(u32::MAX)),
                icons: None,
                meta: None,
            },
            annotations: Some(Annotations {
                audience: Some(vec![Role::Assistant]),
                priority: Some(0.8),
                last_modified: None,
            }),
        })
        .collect()
}

/// Read a specific resource by URI.
///
/// Returns the markdown content for the matching resource, or an error if
/// the URI is not recognized.
#[must_use]
pub fn read_resource(uri: &str) -> ResourceResult {
    let result = RESOURCES
        .iter()
        .find(|entry| entry.uri == uri)
        .map(|entry| ReadResourceResult {
            contents: vec![ResourceContents::TextResourceContents {
                uri: entry.uri.into(),
                mime_type: Some("text/markdown".into()),
                text: entry.content.into(),
                meta: None,
            }],
        })
        .ok_or_else(|| ResourceError::NotFound(uri.into()));

    ResourceResult::Immediate(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn generated_resources_are_not_empty() {
        assert!(
            !RESOURCES.is_empty(),
            "build.rs should generate at least one resource"
        );
    }

    #[test]
    fn all_entries_have_required_fields() {
        for entry in RESOURCES {
            assert!(!entry.group.is_empty(), "group should not be empty");
            assert!(!entry.name.is_empty(), "name should not be empty");
            assert!(!entry.title.is_empty(), "title should not be empty");
            assert!(
                !entry.description.is_empty(),
                "description should not be empty for {}",
                entry.name
            );
            assert!(!entry.uri.is_empty(), "uri should not be empty");
            assert!(
                !entry.content.is_empty(),
                "content should not be empty for {}",
                entry.name
            );
        }
    }

    #[test]
    fn uri_format_matches_group_colon_name() {
        for entry in RESOURCES {
            let expected = format!("{}:{}", entry.group, entry.name);
            assert_eq!(
                entry.uri, expected,
                "URI should be group:name for {}",
                entry.name
            );
        }
    }

    #[test]
    fn list_resources_returns_all_entries() {
        let resources = list_resources();
        assert_eq!(resources.len(), RESOURCES.len());
    }

    #[test]
    fn list_resources_sets_annotations() {
        let resources = list_resources();
        for resource in &resources {
            let annotations = resource
                .annotations
                .as_ref()
                .expect("annotations should be set");
            assert_eq!(
                annotations.audience.as_deref(),
                Some(&[Role::Assistant][..])
            );
            assert_eq!(annotations.priority, Some(0.8));
        }
    }

    #[test]
    fn list_resources_sets_mime_type() {
        let resources = list_resources();
        for resource in &resources {
            assert_eq!(resource.raw.mime_type.as_deref(), Some("text/markdown"));
        }
    }

    #[test]
    fn read_resource_returns_content_for_valid_uri() {
        for entry in RESOURCES {
            let result = read_resource(entry.uri);
            let ResourceResult::Immediate(Ok(read_result)) = result else {
                panic!("expected Immediate(Ok(...)) for URI {}", entry.uri);
            };
            assert_eq!(read_result.contents.len(), 1);
            match &read_result.contents[0] {
                ResourceContents::TextResourceContents { uri, text, .. } => {
                    assert_eq!(uri, entry.uri);
                    assert_eq!(text, entry.content);
                }
                ResourceContents::BlobResourceContents { .. } => {
                    panic!("expected text content, got blob");
                }
            }
        }
    }

    #[test]
    fn read_resource_returns_error_for_unknown_uri() {
        let result = read_resource("nonexistent:nothing");
        let ResourceResult::Immediate(Err(ResourceError::NotFound(uri))) = result else {
            panic!("expected Immediate(Err(NotFound(...)))");
        };
        assert_eq!(uri, "nonexistent:nothing");
    }

    #[test]
    fn insights_group_exists() {
        let has_insights = RESOURCES.iter().any(|e| e.group == "insights");
        assert!(has_insights, "should have at least one insights resource");
    }

    #[test]
    fn known_insight_resources_present() {
        let names: Vec<&str> = RESOURCES
            .iter()
            .filter(|e| e.group == "insights")
            .map(|e| e.name)
            .collect();
        assert!(
            names.contains(&"shaded-views"),
            "should have shaded-views insight"
        );
        assert!(
            names.contains(&"sketch-constraints"),
            "should have sketch-constraints insight"
        );
    }
}
