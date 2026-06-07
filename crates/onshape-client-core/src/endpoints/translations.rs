use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::endpoints::shared::{ElementRef, Error, element_path, encode_path_segment, json_post};
use crate::request::{ApiRequest, ApiResponse};
use http::{HeaderMap, Method};

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
