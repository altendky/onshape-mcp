use serde::Serialize;

use crate::endpoints::shared::{ElementRef, Error, element_path, json_post};
use crate::request::ApiRequest;

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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use http::Method;
    use serde::Serialize;
    use serde_json::json;

    use super::*;
    use crate::endpoints::shared::{JSON_CONTENT_TYPE, WorkspaceVersion};
    use crate::request::RequestBody;

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
}
