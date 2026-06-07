//! Pure request and response helpers for Onshape API endpoints.
//!
//! These helpers build [`ApiRequest`] values and parse response data without
//! performing network, filesystem, clock, storage, or runtime I/O.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::request::{ApiRequest, ApiResponse, RequestBody};
use http::{HeaderMap, Method};

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

    const fn token(self) -> &'static str {
        match self {
            Self::Workspace(_) => "w",
            Self::Version(_) => "v",
            Self::Microversion(_) => "m",
        }
    }

    const fn id(self) -> &'a str {
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

/// Advanced export options shared by typed export request bodies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportAdvancedParams<'a> {
    /// URL-encoded configuration query string, separated by `;` for multiple
    /// values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<&'a str>,
}

/// Mesh tessellation options for typed mesh export request bodies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMeshParams<'a> {
    /// Maximum angular deviation between analytical surfaces and triangulation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angular_tolerance: Option<f64>,
    /// Maximum distance deviation between analytical surfaces and triangulation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_tolerance: Option<f64>,
    /// Maximum triangle edge length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_chord_length: Option<f64>,
    /// Export resolution, such as `fine`, `medium`, or `coarse`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<&'a str>,
    /// Export unit, using Onshape's `GBTExportUnit` wire value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'a str>,
}

/// Typed request body for glTF export endpoints.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GltfExportRequestBody<'a> {
    /// Advanced export options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced_params: Option<ExportAdvancedParams<'a>>,
    /// The name of the exported file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_name: Option<&'a str>,
    /// Whether to exclude hidden parts from export.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_hidden_entities: Option<bool>,
    /// Whether parts should be exported as a group or individually in a zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<bool>,
    /// Whether topology IDs should be exported as attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_export_ids: Option<bool>,
    /// Rotate model from Z-axis-up orientation to Y-axis-up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_y_axis_up: Option<bool>,
    /// Mesh tessellation options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_params: Option<ExportMeshParams<'a>>,
    /// Send notification to the user client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_user: Option<bool>,
    /// Create a blob with exported file in the source document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_in_document: Option<bool>,
    /// Automatically download a translated file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_auto_download: Option<bool>,
}

/// Typed request body for STEP export endpoints.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepExportRequestBody<'a> {
    /// Advanced export options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced_params: Option<ExportAdvancedParams<'a>>,
    /// The name of the exported file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_name: Option<&'a str>,
    /// Whether to exclude hidden parts from export.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_hidden_entities: Option<bool>,
    /// Whether parts should be exported as a group or individually in a zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<bool>,
    /// Whether topology IDs should be exported as attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_export_ids: Option<bool>,
    /// Rotate model from Z-axis-up orientation to Y-axis-up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_y_axis_up: Option<bool>,
    /// Send notification to the user client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_user: Option<bool>,
    /// Original geometry processing mode before STEP translation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_parasolid_preprocessing_option: Option<&'a str>,
    /// Export unit, using Onshape's `GBTExportUnit` wire value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_unit: Option<&'a str>,
    /// STEP version string, such as `AP242`, `AP203`, or `AP214`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_version_string: Option<&'a str>,
    /// Create a blob with exported file in the source document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_in_document: Option<bool>,
    /// Automatically download a translated file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_auto_download: Option<bool>,
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

/// Optional query parameters for [`get_configuration`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GetConfigurationOptions<'a> {
    /// Linked document ID used when accessing linked version data.
    pub link_document_id: Option<&'a str>,
}

/// Optional query parameters for [`encode_configuration_map`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeConfigurationMapOptions<'a> {
    /// Version ID to use while encoding the configuration map.
    pub version_id: Option<&'a str>,
    /// Linked document ID used when accessing linked version data.
    pub link_document_id: Option<&'a str>,
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
        method: Method::GET,
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
        method: Method::GET,
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
        method: Method::GET,
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

/// Build a configuration definition request.
///
/// `OpenAPI` operation ID: `getConfiguration`.
#[must_use]
pub fn get_configuration(
    target: WvmElementRef<'_>,
    options: GetConfigurationOptions<'_>,
) -> ApiRequest {
    let mut query_params = Vec::new();
    if let Some(link_document_id) = options.link_document_id {
        query_params.push(("linkDocumentId".to_string(), link_document_id.to_string()));
    }

    ApiRequest {
        method: Method::GET,
        path: wvm_element_path("elements", target, "/configuration"),
        query_params,
        headers: HeaderMap::new(),
        body: None,
        content_type: None,
    }
}

/// Build a configuration encoding request.
///
/// `OpenAPI` operation ID: `encodeConfigurationMap`.
///
/// # Errors
///
/// Returns an error if `params` cannot be serialized as a JSON object.
pub fn encode_configuration_map<P: Serialize + ?Sized>(
    document_id: &str,
    element_id: &str,
    options: EncodeConfigurationMapOptions<'_>,
    params: &P,
) -> Result<ApiRequest, Error> {
    let mut request = json_post(
        format!(
            "/elements/d/{}/e/{}/configurationencodings",
            encode_path_segment(document_id),
            encode_path_segment(element_id)
        ),
        params,
    )?;

    if let Some(version_id) = options.version_id {
        request
            .query_params
            .push(("versionId".to_string(), version_id.to_string()));
    }
    if let Some(link_document_id) = options.link_document_id {
        request
            .query_params
            .push(("linkDocumentId".to_string(), link_document_id.to_string()));
    }

    Ok(request)
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
        method: Method::POST,
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

fn wvm_element_path(kind: &str, target: WvmElementRef<'_>, suffix: &str) -> String {
    format!(
        "/{kind}/d/{}/{}/{}/e/{}{}",
        encode_path_segment(target.document_id),
        target.workspace_version_or_microversion.token(),
        encode_path_segment(target.workspace_version_or_microversion.id()),
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
        assert_eq!(request.method, Method::POST);
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
    fn gltf_export_request_body_serializes_golden_json() {
        let body = GltfExportRequestBody {
            advanced_params: Some(ExportAdvancedParams {
                configuration: Some("size%3Dlarge"),
            }),
            destination_name: Some("preview-model"),
            exclude_hidden_entities: Some(true),
            grouping: Some(false),
            include_export_ids: Some(true),
            is_y_axis_up: Some(true),
            mesh_params: Some(ExportMeshParams {
                angular_tolerance: Some(0.001),
                distance_tolerance: Some(0.002),
                maximum_chord_length: Some(0.01),
                resolution: Some("fine"),
                unit: Some("METER"),
            }),
            notify_user: Some(false),
            store_in_document: Some(false),
            trigger_auto_download: Some(false),
        };

        assert_eq!(
            serde_json::to_value(&body).expect("body should serialize"),
            json!({
                "advancedParams": {
                    "configuration": "size%3Dlarge"
                },
                "destinationName": "preview-model",
                "excludeHiddenEntities": true,
                "grouping": false,
                "includeExportIds": true,
                "isYAxisUp": true,
                "meshParams": {
                    "angularTolerance": 0.001,
                    "distanceTolerance": 0.002,
                    "maximumChordLength": 0.01,
                    "resolution": "fine",
                    "unit": "METER"
                },
                "notifyUser": false,
                "storeInDocument": false,
                "triggerAutoDownload": false
            })
        );
    }

    #[test]
    fn step_export_request_body_serializes_golden_json() {
        let body = StepExportRequestBody {
            advanced_params: Some(ExportAdvancedParams {
                configuration: Some("size%3Dlarge"),
            }),
            destination_name: Some("step-model"),
            exclude_hidden_entities: Some(true),
            grouping: Some(true),
            include_export_ids: Some(false),
            is_y_axis_up: Some(false),
            notify_user: Some(false),
            step_parasolid_preprocessing_option: Some("NO_PRE_PROCESSING"),
            step_unit: Some("MILLIMETER"),
            step_version_string: Some("AP242"),
            store_in_document: Some(false),
            trigger_auto_download: Some(false),
        };

        assert_eq!(
            serde_json::to_value(&body).expect("body should serialize"),
            json!({
                "advancedParams": {
                    "configuration": "size%3Dlarge"
                },
                "destinationName": "step-model",
                "excludeHiddenEntities": true,
                "grouping": true,
                "includeExportIds": false,
                "isYAxisUp": false,
                "notifyUser": false,
                "stepParasolidPreprocessingOption": "NO_PRE_PROCESSING",
                "stepUnit": "MILLIMETER",
                "stepVersionString": "AP242",
                "storeInDocument": false,
                "triggerAutoDownload": false
            })
        );
    }

    #[test]
    fn typed_export_request_bodies_omit_unset_fields() {
        assert_eq!(
            serde_json::to_value(GltfExportRequestBody::default()).expect("body should serialize"),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(StepExportRequestBody::default()).expect("body should serialize"),
            json!({})
        );
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

        assert_eq!(request.method, Method::GET);
        assert_eq!(request.path, "/translations/translation%2F1");
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn get_all_translator_formats_builds_golden_request() {
        let request = get_all_translator_formats();

        assert_eq!(request.method, Method::GET);
        assert_eq!(request.path, "/translations/translationformats");
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn download_external_data_builds_golden_request() {
        let request = download_external_data("doc/1", "file name.step");

        assert_eq!(request.method, Method::GET);
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
    fn get_configuration_workspace_builds_golden_request_without_query() {
        let target = WvmElementRef::new(
            "doc",
            WorkspaceVersionMicroversion::workspace("workspace"),
            "element",
        );

        let request = get_configuration(target, GetConfigurationOptions::default());

        assert_eq!(request.method, Method::GET);
        assert_eq!(
            request.path,
            "/elements/d/doc/w/workspace/e/element/configuration"
        );
        assert!(request.query_params.is_empty());
        assert!(request.body.is_none());
        assert!(request.content_type.is_none());
    }

    #[test]
    fn get_configuration_version_builds_golden_request_with_query() {
        let target = WvmElementRef::new(
            "doc",
            WorkspaceVersionMicroversion::version("version"),
            "element",
        );

        let request = get_configuration(
            target,
            GetConfigurationOptions {
                link_document_id: Some("linked-doc"),
            },
        );

        assert_eq!(
            request.path,
            "/elements/d/doc/v/version/e/element/configuration"
        );
        assert_eq!(
            request.query_params,
            [("linkDocumentId".to_string(), "linked-doc".to_string())]
        );
    }

    #[test]
    fn get_configuration_microversion_percent_encodes_selector_ids() {
        let target = WvmElementRef::new(
            "doc/1",
            WorkspaceVersionMicroversion::microversion("micro+1"),
            "elem 1",
        );

        let request = get_configuration(target, GetConfigurationOptions::default());

        assert_eq!(
            request.path,
            "/elements/d/doc%2F1/m/micro%2B1/e/elem%201/configuration"
        );
    }

    #[test]
    fn encode_configuration_map_omits_none_query_params() {
        let request = encode_configuration_map(
            "doc",
            "element",
            EncodeConfigurationMapOptions::default(),
            &json!({ "parameters": [] }),
        )
        .expect("request should build");

        assert_eq!(request.method, Method::POST);
        assert_eq!(
            request.path,
            "/elements/d/doc/e/element/configurationencodings"
        );
        assert!(request.query_params.is_empty());
        assert_eq!(request.content_type.as_deref(), Some(JSON_CONTENT_TYPE));
        assert!(
            request
                .body
                .and_then(|body| body.as_json().cloned())
                .is_some()
        );
    }

    #[test]
    fn encode_configuration_map_includes_query_params_in_stable_order() {
        let request = encode_configuration_map(
            "doc/1",
            "elem+1",
            EncodeConfigurationMapOptions {
                version_id: Some("version"),
                link_document_id: Some("linked-doc"),
            },
            &json!({ "parameters": [] }),
        )
        .expect("request should build");

        assert_eq!(
            request.path,
            "/elements/d/doc%2F1/e/elem%2B1/configurationencodings"
        );
        assert_eq!(
            request.query_params,
            [
                ("versionId".to_string(), "version".to_string()),
                ("linkDocumentId".to_string(), "linked-doc".to_string()),
            ]
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
