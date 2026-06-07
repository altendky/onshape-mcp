use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::endpoints::shared::{ElementRef, Error, element_path, encode_path_segment, json_post};
use crate::request::{ApiRequest, ApiResponse};
use http::{HeaderMap, Method};

/// Mesh tessellation options for generic translation request bodies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationMeshParams<'a> {
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

/// Typed request body for generic mesh translation endpoints, including 3MF.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshTranslationRequestBody<'a> {
    /// The name of the exported file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_name: Option<&'a str>,
    /// Whether to exclude hidden parts from export.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_hidden_entities: Option<bool>,
    /// The name of the file format, such as `3MF`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_name: Option<&'a str>,
    /// Mesh tessellation options. These serialize as flat translation fields.
    #[serde(flatten)]
    pub mesh_params: TranslationMeshParams<'a>,
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::*;
    use crate::endpoints::shared::{JSON_CONTENT_TYPE, WorkspaceVersion};
    use crate::request::RequestBody;
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
    fn mesh_translation_request_body_serializes_golden_json() {
        let body = MeshTranslationRequestBody {
            destination_name: Some("coarse-model"),
            exclude_hidden_entities: Some(true),
            format_name: Some("3MF"),
            mesh_params: TranslationMeshParams {
                angular_tolerance: Some(0.5),
                distance_tolerance: Some(0.01),
                maximum_chord_length: Some(0.1),
                resolution: Some("coarse"),
                unit: Some("MILLIMETER"),
            },
            notify_user: Some(false),
            store_in_document: Some(false),
            trigger_auto_download: Some(false),
        };

        assert_eq!(
            serde_json::to_value(&body).expect("body should serialize"),
            json!({
                "angularTolerance": 0.5,
                "destinationName": "coarse-model",
                "distanceTolerance": 0.01,
                "excludeHiddenEntities": true,
                "formatName": "3MF",
                "maximumChordLength": 0.1,
                "notifyUser": false,
                "resolution": "coarse",
                "storeInDocument": false,
                "triggerAutoDownload": false,
                "unit": "MILLIMETER"
            })
        );
    }

    #[test]
    fn typed_mesh_translation_body_omits_unset_fields() {
        assert_eq!(
            serde_json::to_value(MeshTranslationRequestBody::default())
                .expect("body should serialize"),
            json!({})
        );
    }

    #[test]
    fn create_part_studio_translation_accepts_typed_mesh_body() {
        let body = MeshTranslationRequestBody {
            destination_name: Some("export-name"),
            format_name: Some("3MF"),
            mesh_params: TranslationMeshParams {
                angular_tolerance: Some(0.5),
                distance_tolerance: Some(0.01),
                maximum_chord_length: Some(0.1),
                resolution: Some("coarse"),
                unit: Some("MILLIMETER"),
            },
            notify_user: Some(false),
            store_in_document: Some(false),
            ..MeshTranslationRequestBody::default()
        };

        let request =
            create_part_studio_translation(target(), &body).expect("request should build");

        assert_json_post(
            &request,
            "/partstudios/d/doc%2F1/v/ver%201/e/elem%2B1/translations",
            "3MF",
        );
        let body = request
            .body
            .as_ref()
            .and_then(RequestBody::as_json)
            .expect("request should have a JSON body");
        assert_eq!(body["angularTolerance"], 0.5);
        assert_eq!(body["resolution"], "coarse");
        assert_eq!(body["unit"], "MILLIMETER");
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
