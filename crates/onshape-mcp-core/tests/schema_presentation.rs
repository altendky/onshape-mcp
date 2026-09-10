#![allow(clippy::expect_used, clippy::panic)]

use onshape_mcp_core::{
    ValidationState,
    config::ResolvedAuth,
    tools::api::{self, Effect, Policy, ToolKind},
    tools::{self, ToolEffect},
};
use onshape_openapi::OpenApiSpec;
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

const NEUTRAL_POLICY: Policy = Policy {
    present_endpoint: |_, _| {},
    present_schema: |_, _| {},
    validate_body: |_, _, _| Ok(()),
    validate_request: |_| Ok(()),
    append_error_details: |_, _| {},
};

fn specification() -> OpenApiSpec {
    let pet_ref = json!({"$ref": "#/components/schemas/Pet"});
    let properties = json!({
        "pet": pet_ref,
        "pets": {"type": "array", "items": pet_ref},
        "label": {"type": "string", "x-example": "retained"},
        "empty": {"$ref": "#/components/schemas/Empty"},
        "plain": {"$ref": "#/components/schemas/Plain"},
        "missing": {"$ref": "#/components/schemas/Missing"},
        "external": {"$ref": "https://example.com/pet.json"},
        "nested": {"type": "object", "properties": {"pet": pet_ref}},
        "sourceAnnotation": {"type": "string", "x-bttype-options": ["source-value"]}
    });
    let media = |name: &str| {
        json!({
            "application/json": {"schema": {"$ref": format!("#/components/schemas/{name}")}}
        })
    };
    OpenApiSpec::from_value(&json!({
        "openapi": "3.0.1",
        "info": {"title": "Pet API", "version": "1.0"},
        "servers": [{"url": "https://example.com"}],
        "paths": {
            "/pets": {"post": {
                "operationId": "createPet",
                "requestBody": {"content": media("Envelope")},
                "responses": {"200": {"description": "A pet", "content": media("Composed")}}
            }},
            "/empty": {"get": {
                "operationId": "empty",
                "responses": {"204": {"description": "No content"}}
            }},
            "/boolean": {"get": {
                "operationId": "booleanSchema",
                "responses": {"200": {"description": "Any value", "content": {
                    "application/json": {"schema": true}
                }}}
            }}
        },
        "components": {"schemas": {
            "Pet": {
                "type": "object",
                "properties": {"kind": {"type": "string"}},
                "discriminator": {
                    "propertyName": "kind",
                    "mapping": {
                        "dog": "#/components/schemas/Dog",
                        "cat": "#/components/schemas/Cat"
                    }
                }
            },
            "Cat": {"allOf": [pet_ref]},
            "Dog": {"allOf": [pet_ref]},
            "Empty": {"discriminator": {"propertyName": "kind", "mapping": {}}},
            "Plain": {"type": "object"},
            "Envelope": {"type": "object", "properties": properties},
            "Composed": {"allOf": [
                {"properties": properties},
                {"description": "No properties"},
                true
            ]}
        }}
    }))
    .expect("valid specification")
}

fn onshape_result(spec: &OpenApiSpec, name: &str, arguments: &Value) -> CallToolResult {
    let ToolEffect::Done(result) = tools::call_tool(
        name,
        arguments.as_object(),
        &ResolvedAuth::Basic,
        &ValidationState::default(),
        Some(spec),
    ) else {
        panic!("schema presentation must not produce I/O");
    };
    result.expect("tool result")
}

#[test]
fn onshape_schema_presentation_matches_existing_output() {
    let spec = specification();
    let calls = [
        (
            "explain",
            "onshape_api_explain",
            json!({"endpoint": "createPet"}),
        ),
        ("empty", "onshape_api_explain", json!({"endpoint": "empty"})),
        (
            "boolean",
            "onshape_api_explain",
            json!({"endpoint": "booleanSchema"}),
        ),
        (
            "schema",
            "onshape_api_schema",
            json!({"schema": "Envelope"}),
        ),
        (
            "composed",
            "onshape_api_schema",
            json!({"schema": "Composed"}),
        ),
        ("subtypes", "onshape_api_schema", json!({"schema": "Pet"})),
        (
            "emptyMapping",
            "onshape_api_schema",
            json!({"schema": "Empty"}),
        ),
    ];
    let actual: serde_json::Map<String, Value> = calls
        .into_iter()
        .map(|(key, name, args)| {
            (
                key.into(),
                serde_json::to_value(onshape_result(&spec, name, &args)).expect("JSON result"),
            )
        })
        .collect();
    // Captured from the public Onshape dispatcher before moving annotations
    // out of the parser. Compare the entire MCP result, including text encoding.
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/onshape-schema-presentation.json"))
            .expect("valid baseline snapshot");
    assert_eq!(Value::Object(actual), expected);
}

fn generic_result(
    spec: &OpenApiSpec,
    kind: ToolKind,
    arguments: &Value,
    policy: &Policy,
) -> CallToolResult {
    let Effect::Done(result) = api::dispatch(kind, arguments.as_object(), spec, policy) else {
        panic!("schema presentation must not produce I/O");
    };
    result.expect("tool result")
}

fn detail(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(false));
    serde_json::from_str(&result.content[0].as_text().expect("text content").text)
        .expect("JSON detail")
}

#[test]
fn generic_tools_preserve_source_schemas_before_and_after_onshape_presentation() {
    let spec = specification();
    let endpoint = spec.explain("createPet").expect("endpoint");
    let schema = spec.lookup_schema("Envelope").expect("schema");
    let properties = &schema.properties;
    assert_eq!(
        properties["pet"],
        json!({"$ref": "#/components/schemas/Pet"})
    );
    assert_eq!(properties["pets"]["items"], properties["pet"]);
    assert_eq!(
        properties["sourceAnnotation"]["x-bttype-options"],
        json!(["source-value"])
    );
    assert_eq!(
        endpoint.request_body_schema.as_ref().expect("request")["properties"],
        *properties
    );
    assert_eq!(
        endpoint.response_schema.as_ref().expect("response")["allOf"][0]["properties"],
        *properties
    );

    for _ in 0..2 {
        assert_eq!(
            detail(&generic_result(
                &spec,
                ToolKind::Explain,
                &json!({"endpoint": "createPet"}),
                &NEUTRAL_POLICY
            )),
            serde_json::to_value(&endpoint).expect("endpoint JSON")
        );
        assert_eq!(
            detail(&generic_result(
                &spec,
                ToolKind::Schema,
                &json!({"schema": "Envelope"}),
                &NEUTRAL_POLICY
            )),
            serde_json::to_value(&schema).expect("schema JSON")
        );
        let presented = detail(&onshape_result(
            &spec,
            "onshape_api_explain",
            &json!({"endpoint": "createPet"}),
        ));
        assert_eq!(
            presented["request_body_schema"]["properties"]["pet"]["x-bttype-options"],
            json!(["cat", "dog"])
        );
        let presented = detail(&onshape_result(
            &spec,
            "onshape_api_schema",
            &json!({"schema": "Envelope"}),
        ));
        assert_eq!(
            presented["properties"]["pets"]["items"]["x-bttype-options"],
            json!(["cat", "dog"])
        );
    }
}

#[test]
fn alternate_host_can_present_both_detail_types() {
    let spec = specification();
    let policy = Policy {
        present_endpoint: |detail, spec| {
            assert!(spec.lookup_schema("Pet").is_ok());
            detail.description = "Host endpoint guidance".into();
        },
        present_schema: |detail, spec| {
            assert!(spec.explain("createPet").is_ok());
            detail.description = Some("Host schema guidance".into());
        },
        ..NEUTRAL_POLICY
    };
    let endpoint = detail(&generic_result(
        &spec,
        ToolKind::Explain,
        &json!({"endpoint": "createPet"}),
        &policy,
    ));
    assert_eq!(endpoint["description"], "Host endpoint guidance");
    let schema = detail(&generic_result(
        &spec,
        ToolKind::Schema,
        &json!({"schema": "Envelope"}),
        &policy,
    ));
    assert_eq!(schema["description"], "Host schema guidance");
    assert_eq!(
        endpoint["request_body_schema"]["properties"],
        schema["properties"]
    );
    assert_eq!(
        schema["properties"]["pet"],
        json!({"$ref": "#/components/schemas/Pet"})
    );
}

#[test]
fn invalid_arguments_and_missing_entries_skip_presentation() {
    let spec = specification();
    let policy = Policy {
        present_endpoint: |_, _| panic!("invalid explain must skip presentation"),
        present_schema: |_, _| panic!("invalid lookup must skip presentation"),
        ..NEUTRAL_POLICY
    };
    for (kind, args) in [
        (ToolKind::Explain, json!({})),
        (ToolKind::Explain, json!({"endpoint": "missing"})),
        (ToolKind::Schema, json!({})),
        (ToolKind::Schema, json!({"schema": "Missing"})),
    ] {
        assert_eq!(
            generic_result(&spec, kind, &args, &policy).is_error,
            Some(true)
        );
    }
}
