use serde::Serialize;

use crate::endpoints::shared::{
    Error, WvmElementRef, encode_path_segment, json_post, wvm_element_path,
};
use crate::request::ApiRequest;
use http::{HeaderMap, Method};

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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::endpoints::shared::{JSON_CONTENT_TYPE, WorkspaceVersionMicroversion};

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
}
