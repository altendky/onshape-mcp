use crate::endpoints::shared::encode_path_segment;
use crate::request::ApiRequest;
use http::{HeaderMap, Method};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
