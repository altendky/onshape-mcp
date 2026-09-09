//! Onshape request validation and diagnostic interpretation for MCP tools.
//!
//! These helpers operate only on data. Tool dispatch, generic request and file
//! handling, and HTTP status classification stay in the parent module.

use std::collections::{HashMap, HashSet};

use onshape_client_core::request::{ApiRequest, RequestBody};
use serde_json::{Map, Value};

use super::FileReference;

/// Apply Onshape endpoint validation before request construction and file reads.
///
/// # Errors
///
/// Returns the existing document validation diagnostic when a `createDocument`
/// body is invalid, allowing fields whose contents will be supplied by files.
pub fn validate_call_body(
    endpoint: &str,
    body: Option<&Value>,
    file_refs: &[FileReference],
) -> Result<(), String> {
    if endpoint == "createDocument" {
        validate_create_document_body_before_file_injection(body, file_refs)
    } else {
        Ok(())
    }
}

/// Revalidate an Onshape request after file contents have been injected.
///
/// # Errors
///
/// Returns a document validation diagnostic for an invalid JSON POST to
/// `/documents`. Other requests are accepted without endpoint-specific checks.
pub fn validate_injected_request(request: &ApiRequest) -> Result<(), String> {
    if is_create_document_request(request) {
        let body = request.body.as_ref().and_then(RequestBody::as_json);
        validate_create_document_body(body)
    } else {
        Ok(())
    }
}

/// Append allowlisted Onshape error details to an existing HTTP diagnostic.
/// Raw response text, unrecognized codes, and arbitrary severity values are omitted.
pub fn append_api_error_details(detail: &mut String, body: &[u8]) {
    use std::fmt::Write;

    if let Ok(body) = serde_json::from_slice::<Value>(body) {
        if let Some(code) = safe_onshape_error_code(&body) {
            let _ = write!(detail, "; error_code={code}");
        }
        if let Some(severity) = safe_onshape_error_severity(&body) {
            let _ = write!(detail, "; severity={severity}");
        }
    }
}

/// Name the JSON value type without revealing its contents.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Retain known Onshape field names and redact unrecognized names.
fn safe_diagnostic_field_name(name: &str) -> &str {
    match name {
        "body"
        | "code"
        | "description"
        | "details"
        | "elements"
        | "error"
        | "errorCode"
        | "forceExportRules"
        | "generateUnknownMessages"
        | "isEmptyContent"
        | "isPublic"
        | "message"
        | "moreInfoUrl"
        | "name"
        | "notes"
        | "notRevisionManaged"
        | "oldClientNotes"
        | "ownerEmail"
        | "ownerId"
        | "ownerType"
        | "parentId"
        | "projectId"
        | "requestId"
        | "retryable"
        | "status"
        | "statusCode"
        | "statusMsg"
        | "tags" => name,
        _ => "<unrecognized field>",
    }
}

/// Describe body structure using only allowlisted field names and value types.
fn json_shape(value: &Value) -> String {
    match value {
        Value::Object(fields) => {
            let mut fields: Vec<_> = fields
                .iter()
                .map(|(name, value)| {
                    format!(
                        "{}: {}",
                        safe_diagnostic_field_name(name),
                        json_type_name(value)
                    )
                })
                .collect();
            fields.sort();
            fields.dedup();
            if fields.is_empty() {
                "object with no fields".to_string()
            } else {
                format!("object with fields {{{}}}", fields.join(", "))
            }
        }
        Value::Array(items) => {
            let mut item_types: Vec<_> = items.iter().map(json_type_name).collect();
            item_types.sort_unstable();
            item_types.dedup();
            if item_types.is_empty() {
                "array with no items".to_string()
            } else {
                format!("array with item types {{{}}}", item_types.join(", "))
            }
        }
        _ => json_type_name(value).to_string(),
    }
}

const CREATE_DOCUMENT_STRING_FIELDS: &[&str] = &[
    "description",
    "notes",
    "oldClientNotes",
    "ownerEmail",
    "ownerId",
    "parentId",
    "projectId",
];
const CREATE_DOCUMENT_BOOLEAN_FIELDS: &[&str] = &[
    "forceExportRules",
    "generateUnknownMessages",
    "isEmptyContent",
    "isPublic",
    "notRevisionManaged",
];

/// Collect type errors for known optional document fields without echoing values.
fn create_document_invalid_fields(fields: &Map<String, Value>) -> Vec<String> {
    let mut invalid = Vec::new();
    for field in CREATE_DOCUMENT_STRING_FIELDS {
        if let Some(value) = fields.get(*field)
            && !value.is_null()
            && !value.is_string()
        {
            invalid.push(format!(
                "{field} must be string or null, received {}",
                json_type_name(value)
            ));
        }
    }
    for field in CREATE_DOCUMENT_BOOLEAN_FIELDS {
        if let Some(value) = fields.get(*field)
            && !value.is_null()
            && !value.is_boolean()
        {
            invalid.push(format!(
                "{field} must be boolean or null, received {}",
                json_type_name(value)
            ));
        }
    }
    if let Some(value) = fields.get("ownerType")
        && !value.is_null()
        && !value
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64())
    {
        invalid.push(format!(
            "ownerType must be integer or null, received {}",
            json_type_name(value)
        ));
    }
    if let Some(value) = fields.get("elements")
        && !value.is_null()
        && !value.is_array()
    {
        invalid.push(format!(
            "elements must be array or null, received {}",
            json_type_name(value)
        ));
    }
    if let Some(value) = fields.get("tags")
        && !value.is_null()
    {
        match value.as_array() {
            Some(tags) if tags.iter().all(Value::is_string) => {}
            Some(_) => invalid.push("tags must contain only strings".to_string()),
            None => invalid.push(format!(
                "tags must be array or null, received {}",
                json_type_name(value)
            )),
        }
    }
    invalid
}

/// Check the required document name and known field types after body decoding.
fn validate_create_document_body(body: Option<&Value>) -> Result<(), String> {
    let Some(body) = body else {
        return Err("createDocument requires a body containing a non-blank name".to_string());
    };
    let Some(fields) = body.as_object() else {
        let double_encoded = body.as_str().is_some_and(|text| {
            serde_json::from_str::<Value>(text).is_ok_and(|decoded| decoded.is_object())
        });
        let hint = if double_encoded {
            "; the parsed body is a string containing JSON, so it is double-encoded"
        } else {
            ""
        };
        return Err(format!(
            "createDocument body must parse directly to a JSON object; received {}{hint}",
            json_shape(body)
        ));
    };

    match fields.get("name") {
        None => {
            return Err(format!(
                "createDocument body is missing the semantically required name field; received {}",
                json_shape(body)
            ));
        }
        Some(Value::String(name)) if name.trim().is_empty() => {
            return Err(format!(
                "createDocument name must not be blank; received {}",
                json_shape(body)
            ));
        }
        Some(Value::String(_)) => {}
        Some(value) => {
            return Err(format!(
                "createDocument field name must be a string; received {} in {}",
                json_type_name(value),
                json_shape(body)
            ));
        }
    }

    let mut invalid = create_document_invalid_fields(fields);

    if invalid.is_empty() {
        Ok(())
    } else {
        invalid.sort();
        Err(format!(
            "invalid createDocument body fields: {}; received {}",
            invalid.join("; "),
            json_shape(body)
        ))
    }
}

/// Validate the available document body while deferring fields supplied by files.
fn validate_create_document_body_before_file_injection(
    body: Option<&Value>,
    file_refs: &[FileReference],
) -> Result<(), String> {
    if file_refs.is_empty() {
        return validate_create_document_body(body);
    }

    let Some(body) = body else {
        return validate_create_document_body(None);
    };
    let mut pending_body = body.clone();
    if let Some(fields) = pending_body.as_object_mut() {
        for file_ref in file_refs {
            if file_ref.field == "name" {
                fields.insert(
                    "name".to_string(),
                    Value::String("pending file_ref".to_string()),
                );
            } else {
                fields.remove(&file_ref.field);
            }
        }
    }
    validate_create_document_body(Some(&pending_body))
}

/// Match only JSON POST requests to the document creation path.
fn is_create_document_request(request: &ApiRequest) -> bool {
    request.method == http::Method::POST
        && request.path == "/documents"
        && matches!(request.body.as_ref(), Some(RequestBody::Json(_)))
}

/// Select the first recognized Onshape code from the response diagnostic fields.
fn safe_onshape_error_code(body: &Value) -> Option<&str> {
    let fields = body.as_object()?;
    ["statusEnum", "errorValue", "errorCode", "code"]
        .iter()
        .filter_map(|field| fields.get(*field).and_then(Value::as_str))
        .find(|code| safe_error_code_set().contains(*code))
}

/// Normalize only the fixed Onshape severity values.
fn safe_onshape_error_severity(body: &Value) -> Option<&'static str> {
    let fields = body.as_object()?;
    ["featureStatus", "statusType", "level"]
        .iter()
        .filter_map(|field| fields.get(*field).and_then(Value::as_str))
        .find_map(|severity| match severity {
            "OK" => Some("ok"),
            "INFO" => Some("info"),
            "WARNING" => Some("warning"),
            "ERROR" => Some("error"),
            "UNKNOWN" => Some("unknown"),
            _ => None,
        })
}

/// Embedded JSON mapping of `ErrorStringEnum` values to human-readable messages.
///
/// Generated by `scripts/generate-error-enums.py` from the `FeatureScript`
/// standard library (MIT licensed, Copyright (c) 2013-Present PTC Inc.).
const ERROR_ENUMS_JSON: &str = include_str!("../../error-enums.json");

/// Message lookup and safe diagnostic codes derived from the embedded document.
struct ErrorEnums {
    messages: HashMap<String, String>,
    safe_codes: HashSet<String>,
}

/// Initialize both collections from a single parse on their first use.
#[allow(clippy::expect_used)]
fn error_enums() -> &'static ErrorEnums {
    use std::sync::OnceLock;

    static ENUMS: OnceLock<ErrorEnums> = OnceLock::new();
    ENUMS.get_or_init(|| {
        let parsed: Value =
            serde_json::from_str(ERROR_ENUMS_JSON).expect("embedded error-enums.json is valid");
        let messages = parsed
            .get("enums")
            .and_then(Value::as_object)
            .expect("error-enums.json has an 'enums' object")
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect();
        let safe_codes = parsed
            .get("safe_codes")
            .and_then(Value::as_array)
            .expect("error-enums.json has a 'safe_codes' array")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("safe_codes contains only strings")
                    .to_string()
            })
            .collect();
        ErrorEnums {
            messages,
            safe_codes,
        }
    })
}

/// Lazily parsed error enum mapping.
pub fn error_enum_map() -> &'static HashMap<String, String> {
    &error_enums().messages
}

/// Versioned safe error-code values generated from `FeatureScript` and the
/// bundled `OpenAPI` `GBTErrorStringEnum` schema.
fn safe_error_code_set() -> &'static HashSet<String> {
    &error_enums().safe_codes
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn generated_safe_codes_include_openapi_only_values_and_all_message_codes() {
        let safe_codes = safe_error_code_set();
        assert_eq!(error_enum_map().len(), 1_723);
        assert_eq!(safe_codes.len(), 1_779);
        assert!(safe_codes.contains("CUSTOM_ERROR"));
        assert!(safe_codes.contains("CONFIG_INCORRECT_PARAMETER_TYPE"));
        assert!(safe_codes.contains("TRANSACTION_CONFLICT"));
        assert!(
            error_enum_map()
                .keys()
                .all(|code| safe_codes.contains(code)),
            "every message-bearing FeatureScript code must remain safe"
        );
    }

    #[test]
    fn create_document_request_discriminator_is_exact() {
        let mut request = ApiRequest {
            method: http::Method::POST,
            path: "/documents".to_string(),
            query_params: vec![],
            headers: http::HeaderMap::default(),
            body: Some(RequestBody::Json(serde_json::json!({}))),
            content_type: Some("application/json".to_string()),
        };
        assert!(is_create_document_request(&request));

        request.path = "/documents/search".to_string();
        assert!(!is_create_document_request(&request));
        request.path = "/documents".to_string();
        request.method = http::Method::GET;
        assert!(!is_create_document_request(&request));
        request.method = http::Method::POST;
        request.body = None;
        assert!(!is_create_document_request(&request));
    }
}
