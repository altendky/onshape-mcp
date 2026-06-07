use serde::Serialize;
use serde_json::Value;

use crate::request::{ApiRequest, RequestBody};
use http::{HeaderMap, Method};

pub(super) const JSON_CONTENT_TYPE: &str = "application/json;charset=UTF-8; qs=0.09";

/// Error returned by endpoint helper construction or parsing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Request body serialization failed.
    #[error("failed to serialize endpoint request body as JSON: {0}")]
    SerializeBody(#[source] serde_json::Error),
    /// Request body serialized successfully but was not a JSON object.
    #[error("endpoint request body must serialize to a JSON object")]
    InvalidBodyShape,
    /// Response body parsing failed.
    #[error("failed to parse endpoint response body as JSON: {0}")]
    ParseResponse(#[source] serde_json::Error),
}

/// Workspace-or-version selector for endpoints whose path uses `{wv}/{wvid}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceVersion<'a> {
    /// Workspace selector, serialized in paths as `w/{id}`.
    Workspace(&'a str),
    /// Version selector, serialized in paths as `v/{id}`.
    Version(&'a str),
}

impl<'a> WorkspaceVersion<'a> {
    /// Create a workspace selector.
    #[must_use]
    pub const fn workspace(id: &'a str) -> Self {
        Self::Workspace(id)
    }

    /// Create a version selector.
    #[must_use]
    pub const fn version(id: &'a str) -> Self {
        Self::Version(id)
    }

    pub(super) const fn token(self) -> &'static str {
        match self {
            Self::Workspace(_) => "w",
            Self::Version(_) => "v",
        }
    }

    pub(super) const fn id(self) -> &'a str {
        match self {
            Self::Workspace(id) | Self::Version(id) => id,
        }
    }
}

/// Workspace, version, or microversion selector for endpoints whose path uses
/// `{wvm}/{wvmid}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceVersionMicroversion<'a> {
    /// Workspace selector, serialized in paths as `w/{id}`.
    Workspace(&'a str),
    /// Version selector, serialized in paths as `v/{id}`.
    Version(&'a str),
    /// Microversion selector, serialized in paths as `m/{id}`.
    Microversion(&'a str),
}

impl<'a> WorkspaceVersionMicroversion<'a> {
    /// Create a workspace selector.
    #[must_use]
    pub const fn workspace(id: &'a str) -> Self {
        Self::Workspace(id)
    }

    /// Create a version selector.
    #[must_use]
    pub const fn version(id: &'a str) -> Self {
        Self::Version(id)
    }

    /// Create a microversion selector.
    #[must_use]
    pub const fn microversion(id: &'a str) -> Self {
        Self::Microversion(id)
    }

    pub(super) const fn token(self) -> &'static str {
        match self {
            Self::Workspace(_) => "w",
            Self::Version(_) => "v",
            Self::Microversion(_) => "m",
        }
    }

    pub(super) const fn id(self) -> &'a str {
        match self {
            Self::Workspace(id) | Self::Version(id) | Self::Microversion(id) => id,
        }
    }
}

/// Common document-element target for Part Studio and Assembly export helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementRef<'a> {
    /// Document ID.
    pub document_id: &'a str,
    /// Workspace or version selector.
    pub workspace_or_version: WorkspaceVersion<'a>,
    /// Element ID.
    pub element_id: &'a str,
}

impl<'a> ElementRef<'a> {
    /// Create an element reference.
    #[must_use]
    pub const fn new(
        document_id: &'a str,
        workspace_or_version: WorkspaceVersion<'a>,
        element_id: &'a str,
    ) -> Self {
        Self {
            document_id,
            workspace_or_version,
            element_id,
        }
    }
}

/// Document-element target for endpoints whose path uses `{wvm}/{wvmid}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WvmElementRef<'a> {
    /// Document ID.
    pub document_id: &'a str,
    /// Workspace, version, or microversion selector.
    pub workspace_version_or_microversion: WorkspaceVersionMicroversion<'a>,
    /// Element ID.
    pub element_id: &'a str,
}

impl<'a> WvmElementRef<'a> {
    /// Create a document-element reference for `{wvm}/{wvmid}` endpoints.
    #[must_use]
    pub const fn new(
        document_id: &'a str,
        workspace_version_or_microversion: WorkspaceVersionMicroversion<'a>,
        element_id: &'a str,
    ) -> Self {
        Self {
            document_id,
            workspace_version_or_microversion,
            element_id,
        }
    }
}

pub(super) fn json_post<P: Serialize + ?Sized>(
    path: String,
    params: &P,
) -> Result<ApiRequest, Error> {
    let body: Value = serde_json::to_value(params).map_err(Error::SerializeBody)?;
    if !body.is_object() {
        return Err(Error::InvalidBodyShape);
    }

    Ok(ApiRequest {
        method: Method::POST,
        path,
        query_params: Vec::new(),
        headers: HeaderMap::new(),
        body: Some(RequestBody::Json(body)),
        content_type: Some(JSON_CONTENT_TYPE.to_string()),
    })
}

pub(super) fn element_path(kind: &str, target: ElementRef<'_>, suffix: &str) -> String {
    format!(
        "/{kind}/d/{}/{}/{}/e/{}{}",
        encode_path_segment(target.document_id),
        target.workspace_or_version.token(),
        encode_path_segment(target.workspace_or_version.id()),
        encode_path_segment(target.element_id),
        suffix
    )
}

pub(super) fn wvm_element_path(kind: &str, target: WvmElementRef<'_>, suffix: &str) -> String {
    format!(
        "/{kind}/d/{}/{}/{}/e/{}{}",
        encode_path_segment(target.document_id),
        target.workspace_version_or_microversion.token(),
        encode_path_segment(target.workspace_version_or_microversion.id()),
        encode_path_segment(target.element_id),
        suffix
    )
}

pub(super) fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                String::from(byte as char)
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn json_post_rejects_non_object_bodies() {
        for body in [
            Value::Null,
            json!([]),
            json!("not an object"),
            json!(42),
            json!(true),
        ] {
            assert!(matches!(
                json_post("/path".to_string(), &body),
                Err(Error::InvalidBodyShape)
            ));
        }
    }
}
