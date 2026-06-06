//! Pure request and response helpers for Onshape API endpoints.
//!
//! These helpers build [`ApiRequest`] values and parse response data without
//! performing network, filesystem, clock, storage, or runtime I/O.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::request::{ApiRequest, ApiResponse, HttpMethod, RequestBody};
use http::HeaderMap;

const JSON_CONTENT_TYPE: &str = "application/json;charset=UTF-8; qs=0.09";

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

    const fn token(self) -> &'static str {
        match self {
            Self::Workspace(_) => "w",
            Self::Version(_) => "v",
        }
    }

    const fn id(self) -> &'a str {
        match self {
            Self::Workspace(id) | Self::Version(id) => id,
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

/// Minimal parsed response for `BTTranslationRequestInfo`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequestInfo {
    /// Source document ID, when returned by Onshape.
    #[serde(default)]
    pub document_id: Option<String>,
    /// Failure reason for failed translations, when returned by Onshape.
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Translation request ID.
    #[serde(default)]
    pub id: Option<String>,
    /// Translation request state.
    #[serde(default)]
    pub request_state: Option<TranslationRequestState>,
    /// Result document ID, when the translation creates a document.
    #[serde(default)]
    pub result_document_id: Option<String>,
    /// Result element IDs, when the translation creates elements.
    #[serde(default)]
    pub result_element_ids: Option<Vec<String>>,
    /// Result external data IDs, used with `downloadExternalData`.
    #[serde(default)]
    pub result_external_data_ids: Option<Vec<String>>,
}

impl TranslationRequestInfo {
    /// Return result external data IDs as an empty slice when absent.
    #[must_use]
    pub fn external_data_ids(&self) -> &[String] {
        self.result_external_data_ids.as_deref().unwrap_or_default()
    }

    /// Return result element IDs as an empty slice when absent.
    #[must_use]
    pub fn element_ids(&self) -> &[String] {
        self.result_element_ids.as_deref().unwrap_or_default()
    }
}

/// Onshape translation request states from `BTTranslationRequestState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslationRequestState {
    /// Translation is still active.
    Active,
    /// Translation completed successfully.
    Done,
    /// Translation failed.
    Failed,
    /// A state not present in the currently vendored `OpenAPI` enum.
    Other(String),
}

impl TranslationRequestState {
    /// Return the wire-format state string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "ACTIVE",
            Self::Done => "DONE",
            Self::Failed => "FAILED",
            Self::Other(value) => value,
        }
    }
}

impl Serialize for TranslationRequestState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TranslationRequestState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "ACTIVE" => Self::Active,
            "DONE" => Self::Done,
            "FAILED" => Self::Failed,
            _ => Self::Other(value),
        })
    }
}

/// Build a Part Studio translation request.
///
/// `OpenAPI` operation ID: `createPartStudioTranslation`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_part_studio_translation<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("partstudios", target, "/translations"), params)
}

/// Build an Assembly translation request.
///
/// `OpenAPI` operation ID: `translateFormat`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_assembly_translation<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("assemblies", target, "/translations"), params)
}

/// Build a Part Studio glTF export request.
///
/// `OpenAPI` operation ID: `createPartStudioExportGltf`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_part_studio_export_gltf<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("partstudios", target, "/export/gltf"), params)
}

/// Build a Part Studio STEP export request.
///
/// `OpenAPI` operation ID: `createPartStudioExportStep`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_part_studio_export_step<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("partstudios", target, "/export/step"), params)
}

/// Build an Assembly glTF export request.
///
/// `OpenAPI` operation ID: `createAssemblyExportGltf`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_assembly_export_gltf<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("assemblies", target, "/export/gltf"), params)
}

/// Build an Assembly STEP export request.
///
/// `OpenAPI` operation ID: `createAssemblyExportStep`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn create_assembly_export_step<P: Serialize + ?Sized>(
    target: ElementRef<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    json_post(element_path("assemblies", target, "/export/step"), params)
}

/// Build a translation status request.
///
/// `OpenAPI` operation ID: `getTranslation`.
#[must_use]
pub fn get_translation(translation_id: &str) -> ApiRequest {
    ApiRequest {
        method: HttpMethod::GET,
        path: format!("/translations/{}", encode_path_segment(translation_id)),
        query_params: Vec::new(),
        headers: HeaderMap::new(),
        body: None,
        content_type: None,
    }
}

/// Build a translator formats request.
///
/// `OpenAPI` operation ID: `getAllTranslatorFormats`.
#[must_use]
pub fn get_all_translator_formats() -> ApiRequest {
    ApiRequest {
        method: HttpMethod::GET,
        path: "/translations/translationformats".to_string(),
        query_params: Vec::new(),
        headers: HeaderMap::new(),
        body: None,
        content_type: None,
    }
}

/// Build an external data download request.
///
/// `OpenAPI` operation ID: `downloadExternalData`.
///
/// This helper does not set `If-None-Match` or `Accept` yet. Callers that need
/// cache validation or media negotiation can add request headers before
/// executing the returned request.
#[must_use]
pub fn download_external_data(document_id: &str, foreign_id: &str) -> ApiRequest {
    ApiRequest {
        method: HttpMethod::GET,
        path: format!(
            "/documents/d/{}/externaldata/{}",
            encode_path_segment(document_id),
            encode_path_segment(foreign_id)
        ),
        query_params: Vec::new(),
        headers: HeaderMap::new(),
        body: None,
        content_type: None,
    }
}

/// Parse a translation request info response.
///
/// Response schema: `BTTranslationRequestInfo`.
///
/// # Errors
///
/// Returns an error if the response body is not valid JSON for
/// [`TranslationRequestInfo`]. HTTP status interpretation remains caller-owned.
pub fn parse_translation_request_info(
    response: &ApiResponse,
) -> Result<TranslationRequestInfo, Error> {
    serde_json::from_slice(response.body.as_bytes()).map_err(Error::ParseResponse)
}

fn json_post<P: Serialize + ?Sized>(path: String, params: &P) -> Result<ApiRequest, Error> {
    let body: Value = serde_json::to_value(params).map_err(Error::SerializeBody)?;
    if !body.is_object() {
        return Err(Error::InvalidBodyShape);
    }

    Ok(ApiRequest {
        method: HttpMethod::POST,
        path,
        query_params: Vec::new(),
        headers: HeaderMap::new(),
        body: Some(RequestBody::Json(body)),
        content_type: Some(JSON_CONTENT_TYPE.to_string()),
    })
}

fn element_path(kind: &str, target: ElementRef<'_>, suffix: &str) -> String {
    format!(
        "/{kind}/d/{}/{}/{}/e/{}{}",
        encode_path_segment(target.document_id),
        target.workspace_or_version.token(),
        encode_path_segment(target.workspace_or_version.id()),
        encode_path_segment(target.element_id),
        suffix
    )
}

fn encode_path_segment(value: &str) -> String {
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
    use serde::Serialize;
    use serde_json::json;

    use super::*;
    use crate::request::ResponseBody;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExportParams<'a> {
        format_name: &'a str,
        store_in_document: bool,
        destination_name: &'a str,
    }

    fn target() -> ElementRef<'static> {
        ElementRef::new("doc/1", WorkspaceVersion::version("ver 1"), "elem+1")
    }

    fn assert_json_post(request: &ApiRequest, path: &str, format_name: &str) {
        assert_eq!(request.method, HttpMethod::POST);
        assert_eq!(request.path, path);
        assert!(request.query_params.is_empty());
        assert_eq!(request.content_type.as_deref(), Some(JSON_CONTENT_TYPE));

        let body = request
            .body
            .as_ref()
            .and_then(RequestBody::as_json)
            .expect("request should have a JSON body");

        assert_eq!(body["formatName"], format_name);
        assert_eq!(body["storeInDocument"], false);
        assert_eq!(body["destinationName"], "export-name");
    }

    fn params(format_name: &'static str) -> ExportParams<'static> {
        ExportParams {
            format_name,
            store_in_document: false,
            destination_name: "export-name",
        }
    }

    #[test]
    fn create_part_studio_translation_builds_golden_request() {
        let request =
            create_part_studio_translation(target(), &params("STL")).expect("request should build");

        assert_json_post(
            &request,
            "/partstudios/d/doc%2F1/v/ver%201/e/elem%2B1/translations",
            "STL",
        );
    }

    #[test]
    fn create_assembly_translation_builds_golden_request() {
        let request =
            create_assembly_translation(target(), &params("3MF")).expect("request should build");

        assert_json_post(
            &request,
            "/assemblies/d/doc%2F1/v/ver%201/e/elem%2B1/translations",
            "3MF",
        );
    }

    #[test]
    fn create_part_studio_export_gltf_builds_golden_request() {
        let request = create_part_studio_export_gltf(target(), &params("GLTF"))
            .expect("request should build");

        assert_json_post(
            &request,
            "/partstudios/d/doc%2F1/v/ver%201/e/elem%2B1/export/gltf",
            "GLTF",
        );
    }

    #[test]
    fn create_part_studio_export_step_builds_golden_request() {
        let request = create_part_studio_export_step(target(), &params("STEP"))
            .expect("request should build");

        assert_json_post(
            &request,
            "/partstudios/d/doc%2F1/v/ver%201/e/elem%2B1/export/step",
            "STEP",
        );
    }

    #[test]
    fn create_assembly_export_gltf_builds_golden_request() {
        let request =
            create_assembly_export_gltf(target(), &params("GLTF")).expect("request should build");

        assert_json_post(
            &request,
            "/assemblies/d/doc%2F1/v/ver%201/e/elem%2B1/export/gltf",
            "GLTF",
        );
    }

    #[test]
    fn create_assembly_export_step_builds_golden_request() {
        let request =
            create_assembly_export_step(target(), &params("STEP")).expect("request should build");

        assert_json_post(
            &request,
            "/assemblies/d/doc%2F1/v/ver%201/e/elem%2B1/export/step",
            "STEP",
        );
    }

    #[test]
    fn get_translation_builds_golden_request() {
        let request = get_translation("translation/1");

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(request.path, "/translations/translation%2F1");
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn get_all_translator_formats_builds_golden_request() {
        let request = get_all_translator_formats();

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(request.path, "/translations/translationformats");
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn download_external_data_builds_golden_request() {
        let request = download_external_data("doc/1", "file name.step");

        assert_eq!(request.method, HttpMethod::GET);
        assert_eq!(
            request.path,
            "/documents/d/doc%2F1/externaldata/file%20name.step"
        );
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn workspace_version_workspace_builds_workspace_path() {
        let target = ElementRef::new("doc", WorkspaceVersion::workspace("workspace"), "element");

        let request =
            create_part_studio_translation(target, &params("STEP")).expect("request should build");

        assert_eq!(
            request.path,
            "/partstudios/d/doc/w/workspace/e/element/translations"
        );
    }

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

    #[test]
    fn parse_translation_request_info_reads_export_result() {
        let response = ApiResponse {
            status: http::StatusCode::OK,
            headers: HeaderMap::from_iter([(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(JSON_CONTENT_TYPE),
            )]),
            body: ResponseBody::from(
                json!({
                    "documentId": "doc",
                    "id": "translation",
                    "requestState": "DONE",
                    "resultExternalDataIds": ["external-1"],
                    "resultElementIds": ["element-1"]
                })
                .to_string(),
            ),
        };

        let info = parse_translation_request_info(&response).expect("response should parse");

        assert_eq!(info.document_id.as_deref(), Some("doc"));
        assert_eq!(info.id.as_deref(), Some("translation"));
        assert_eq!(info.request_state, Some(TranslationRequestState::Done));
        assert_eq!(info.external_data_ids(), ["external-1"]);
        assert_eq!(info.element_ids(), ["element-1"]);
    }

    #[test]
    fn parse_translation_request_info_rejects_invalid_json() {
        let response = ApiResponse {
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: ResponseBody::from("not json"),
        };

        assert!(matches!(
            parse_translation_request_info(&response),
            Err(Error::ParseResponse(_))
        ));
    }

    #[test]
    fn parse_translation_request_info_preserves_unknown_state() {
        let response = ApiResponse {
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: ResponseBody::from(json!({ "requestState": "QUEUED" }).to_string()),
        };

        let info = parse_translation_request_info(&response).expect("response should parse");

        assert_eq!(
            info.request_state,
            Some(TranslationRequestState::Other("QUEUED".to_string()))
        );
    }
}
