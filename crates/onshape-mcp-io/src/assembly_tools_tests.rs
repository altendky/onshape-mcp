//! The high-level assembly tools build valid requests against the real
//! embedded Onshape `OpenAPI` specification.

#![allow(clippy::expect_used, clippy::panic)]

use onshape_mcp_core::{ValidationState, config::ResolvedAuth, tools};
use openapi_mcp_spec::request::RequestBody;
use serde_json::{Map, Value, json};

use super::*;

fn spec() -> OpenApiSpec {
    OpenApiSpec::from_json_with_server_url_fallback(OPENAPI_SPEC_JSON, OPENAPI_SERVER_URL_FALLBACK)
        .expect("embedded OpenAPI spec should parse")
}

fn auth() -> ResolvedAuth {
    ResolvedAuth::NotConfigured {
        configured_method: AuthMethod::Auto,
        detail: "test".into(),
    }
}

fn call(name: &str, args: &Value) -> tools::ToolEffect {
    let args: Map<String, Value> = args.as_object().cloned().expect("object");
    tools::call_tool(
        name,
        Some(&args),
        &auth(),
        &ValidationState::default(),
        Some(&spec()),
    )
}

fn expect_request(effect: tools::ToolEffect) -> openapi_mcp_spec::request::ApiRequest {
    match effect {
        tools::ToolEffect::ApiRequest { request, .. } => request,
        other => panic!("expected an API request, got {other:?}"),
    }
}

#[test]
fn assembly_tools_are_listed() {
    let names: Vec<String> = tools::list_tools()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for name in [
        "onshape_assembly_read",
        "onshape_mate",
        "onshape_mate_relation",
        "onshape_mate_connector",
    ] {
        assert!(
            names.iter().any(|n| n == name),
            "{name} missing from {names:?}"
        );
    }
}

#[test]
fn assembly_read_targets_definition_endpoint() {
    let request = expect_request(call(
        "onshape_assembly_read",
        &json!({"did": "D1", "wvm": "v", "wvmid": "V1", "eid": "E1"}),
    ));
    assert_eq!(request.method, http::Method::GET);
    assert!(
        request.path.ends_with("/assemblies/d/D1/v/V1/e/E1"),
        "{}",
        request.path
    );
    assert!(
        request
            .query_params
            .contains(&("includeMateFeatures".into(), "true".into()))
    );
}

#[test]
fn write_tools_post_to_assembly_features() {
    let cases = [
        (
            "onshape_mate",
            json!({"did": "D1", "wid": "W1", "eid": "E1", "name": "Pivot", "mate_type": "REVOLUTE",
                   "connectors": [{"kind": "assembly_feature", "feature_id": "M1"},
                                  {"kind": "part_studio", "path": ["I1"], "feature_id": "F1"}]}),
            "BTMMate-64",
        ),
        (
            "onshape_mate_relation",
            json!({"did": "D1", "wid": "W1", "eid": "E1", "name": "Screw", "relation_type": "SCREW",
                   "mates": [{"feature_id": "M1"}], "distance_per_rev_mm": 1.27}),
            "BTMMateRelation-1412",
        ),
        (
            "onshape_mate_connector",
            json!({"did": "D1", "wid": "W1", "eid": "E1", "name": "Hole", "path": ["I1"],
                   "entity_ids": ["JHK"], "inference": "CENTER"}),
            "BTMMateConnector-66",
        ),
    ];
    for (tool, args, bt_type) in cases {
        let request = expect_request(call(tool, &args));
        assert_eq!(request.method, http::Method::POST, "{tool}");
        assert!(
            request
                .path
                .ends_with("/assemblies/d/D1/w/W1/e/E1/features"),
            "{tool}: {}",
            request.path
        );
        match request.body {
            Some(RequestBody::Json(body)) => {
                assert_eq!(body["feature"]["btType"], bt_type, "{tool}");
            }
            other => panic!("{tool}: expected JSON body, got {other:?}"),
        }
    }
}
