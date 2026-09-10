//! Adapt neutral `OpenAPI` requests to the authenticated Onshape client.

use onshape_client_core::request as client;
use onshape_openapi::request as openapi;

/// Move request data into the Onshape client representation without serialization.
pub fn into_onshape_request(request: openapi::ApiRequest) -> client::ApiRequest {
    let openapi::ApiRequest {
        method,
        path,
        query_params,
        headers,
        body,
        content_type,
    } = request;

    client::ApiRequest {
        method,
        path,
        query_params,
        headers,
        body: body.map(into_onshape_body),
        content_type,
    }
}

/// Preserve JSON values, multipart field order, and binary part metadata.
fn into_onshape_body(body: openapi::RequestBody) -> client::RequestBody {
    match body {
        openapi::RequestBody::Json(value) => client::RequestBody::Json(value),
        openapi::RequestBody::Multipart(openapi::MultipartBody {
            text_fields,
            binary_fields,
        }) => client::RequestBody::Multipart(client::MultipartBody {
            text_fields,
            binary_fields: binary_fields
                .into_iter()
                .map(
                    |openapi::BinaryField {
                         field_name,
                         data,
                         content_type,
                     }| client::BinaryField {
                        field_name,
                        data,
                        content_type,
                    },
                )
                .collect(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue, Method};
    use serde_json::json;

    fn request(
        body: Option<openapi::RequestBody>,
        content_type: Option<&str>,
    ) -> openapi::ApiRequest {
        let mut headers = HeaderMap::new();
        headers.append("x-tag", HeaderValue::from_static("first"));
        headers.append("x-tag", HeaderValue::from_static("second"));
        openapi::ApiRequest {
            method: Method::PATCH,
            path: "/items/a%2Fb".into(),
            query_params: vec![
                ("tag".into(), "first".into()),
                ("tag".into(), "second".into()),
            ],
            headers,
            body,
            content_type: content_type.map(str::to_owned),
        }
    }

    #[test]
    fn conversion_preserves_request_shape_for_all_body_variants() {
        for request in [
            request(None, None),
            request(
                Some(openapi::RequestBody::Json(
                    json!({ "title": "Example", "optional": null }),
                )),
                Some("application/json"),
            ),
            request(
                Some(openapi::RequestBody::Multipart(openapi::MultipartBody {
                    text_fields: vec![
                        ("tag".into(), "first".into()),
                        ("tag".into(), "second".into()),
                    ],
                    binary_fields: vec![
                        openapi::BinaryField {
                            field_name: "file".into(),
                            data: vec![0, 128, 255],
                            content_type: Some("application/octet-stream".into()),
                        },
                        openapi::BinaryField {
                            field_name: "file".into(),
                            data: vec![],
                            content_type: None,
                        },
                    ],
                })),
                Some("multipart/form-data"),
            ),
        ] {
            let expected = serde_json::to_value(&request).expect("serializable neutral request");
            let converted = into_onshape_request(request);
            assert_eq!(
                serde_json::to_value(converted).expect("serializable client request"),
                expected
            );
        }
    }

    #[test]
    fn conversion_preserves_opaque_and_sensitive_header_values() {
        let mut request = request(None, None);
        let mut sensitive = HeaderValue::from_bytes(&[0x80]).expect("valid opaque header");
        sensitive.set_sensitive(true);
        request.headers.append("x-opaque", sensitive);
        request.headers.append(
            "x-opaque",
            HeaderValue::from_bytes(&[0xff]).expect("valid opaque header"),
        );

        let converted = into_onshape_request(request);
        let values: Vec<_> = converted.headers.get_all("x-opaque").iter().collect();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].as_bytes(), &[0x80]);
        assert!(values[0].is_sensitive());
        assert_eq!(values[1].as_bytes(), &[0xff]);
        assert!(!values[1].is_sensitive());
    }
}
