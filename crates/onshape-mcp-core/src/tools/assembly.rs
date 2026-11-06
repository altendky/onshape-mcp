//! High-level assembly tools.
//!
//! Each tool is one Onshape API call:
//!
//! - `onshape_assembly_read` — instances, occurrences (fixed state, position),
//!   mates with the occurrences they join, other assembly features, and Part
//!   Studio mate connectors, in one compact result.
//! - `onshape_mate` — add a mate between two existing mate connectors.
//! - `onshape_mate_relation` — add a screw, gear, rack-and-pinion or linear
//!   relation between mates.
//! - `onshape_mate_connector` — add an assembly mate connector placed by
//!   inference on an entity (hole edge centre, cylinder axis point, face
//!   centroid, ...), with optional flip, offset and rotation.
//!
//! Write tools report the new feature's id and regeneration status from the
//! same response, so a separate verification read is not needed.
//!
//! Payload sources: Onshape's API guide for assemblies (mates, mate
//! connectors, definition layout); `gpambrozio/onshape-mcp` (MIT) for the
//! relation payload and the `secondaryAxisAlignment` parameter that relations
//! need on their mates.

use std::collections::HashMap;
use std::sync::Arc;

use http::HeaderMap;
use openapi_mcp_spec::OpenApiSpec;
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::api::parse_arguments;
use super::{Continuation, ToolEffect, tool_input_error};

/// Tool name: read an assembly.
pub const TOOL_ASSEMBLY_READ: &str = "onshape_assembly_read";
/// Tool name: add a mate.
pub const TOOL_MATE: &str = "onshape_mate";
/// Tool name: add a mate relation.
pub const TOOL_MATE_RELATION: &str = "onshape_mate_relation";
/// Tool name: add an assembly mate connector.
pub const TOOL_MATE_CONNECTOR: &str = "onshape_mate_connector";

/// Names of all tools in this module.
pub const TOOL_NAMES: [&str; 4] = [
    TOOL_ASSEMBLY_READ,
    TOOL_MATE,
    TOOL_MATE_RELATION,
    TOOL_MATE_CONNECTOR,
];

// ============================================================================
// Inputs
// ============================================================================

fn default_wvm() -> String {
    "w".to_string()
}

const fn default_true() -> bool {
    true
}

/// Input for `onshape_assembly_read`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AssemblyReadInput {
    /// Document ID (from the URL after /documents/).
    pub did: String,
    /// "w" (workspace), "v" (version) or "m" (microversion). Default "w".
    #[serde(default = "default_wvm")]
    pub wvm: String,
    /// Workspace, version or microversion ID.
    pub wvmid: String,
    /// Assembly element ID (from the URL after /e/).
    pub eid: String,
    /// Also list Part Studio mate connectors owned by the inserted parts.
    /// Default true.
    #[serde(default = "default_true")]
    pub include_part_studio_connectors: bool,
}

/// Mate types accepted by Onshape's `BTMMate-64` feature.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MateType {
    /// No degrees of freedom.
    Fastened,
    /// Rotation about the connector Z axis.
    Revolute,
    /// Translation along the connector Z axis.
    Slider,
    /// Rotation about and translation along the connector Z axis.
    Cylindrical,
    /// Translation along a slot plus rotation.
    PinSlot,
    /// Planar motion.
    Planar,
    /// Three rotations.
    Ball,
    /// Keeps the Z axes parallel.
    Parallel,
}

impl MateType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Fastened => "FASTENED",
            Self::Revolute => "REVOLUTE",
            Self::Slider => "SLIDER",
            Self::Cylindrical => "CYLINDRICAL",
            Self::PinSlot => "PIN_SLOT",
            Self::Planar => "PLANAR",
            Self::Ball => "BALL",
            Self::Parallel => "PARALLEL",
        }
    }
}

/// A mate connector to join.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectorRef {
    /// A mate connector feature in this assembly (for example one made by
    /// `onshape_mate_connector`).
    AssemblyFeature {
        /// Feature ID of the assembly mate connector.
        feature_id: String,
    },
    /// A mate connector defined in a Part Studio and owned by an inserted
    /// part. Listed by `onshape_assembly_read` under
    /// `part_studio_connectors`.
    PartStudio {
        /// Occurrence path of the instance that carries the connector, top
        /// level first, e.g. `["<subAssemblyInstance>", "<partInstance>"]`.
        path: Vec<String>,
        /// Feature ID of the connector in its Part Studio.
        feature_id: String,
    },
}

/// Input for `onshape_mate`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct MateInput {
    /// Document ID.
    pub did: String,
    /// Workspace ID (writes need a workspace).
    pub wid: String,
    /// Assembly element ID.
    pub eid: String,
    /// Mate name as it should appear in the feature list.
    pub name: String,
    /// Mate type.
    pub mate_type: MateType,
    /// Exactly two connectors. The first one's instance moves relative to
    /// the second's.
    pub connectors: Vec<ConnectorRef>,
    /// Flip the primary (Z) axis alignment. Mates make both connector frames
    /// coincide in the same direction, so two faces in contact (opposite
    /// normals) need `flip: true`.
    #[serde(default)]
    pub flip: bool,
}

/// Relation types accepted by `BTMMateRelation-1412`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationType {
    /// One cylindrical mate: its rotation drives its translation.
    Screw,
    /// Two rotating mates.
    Gear,
    /// A rotating mate (pinion) and a sliding mate (rack).
    RackAndPinion,
    /// Two sliding mates.
    Linear,
}

impl RelationType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Screw => "SCREW",
            Self::Gear => "GEAR",
            Self::RackAndPinion => "RACK_AND_PINION",
            Self::Linear => "LINEAR",
        }
    }

    const fn mate_count(self) -> usize {
        match self {
            Self::Screw => 1,
            Self::Gear | Self::RackAndPinion | Self::Linear => 2,
        }
    }

    const fn uses_length(self) -> bool {
        matches!(self, Self::Screw | Self::RackAndPinion)
    }

    /// Default degree of freedom for the mate at `index`.
    const fn default_dof(self, index: usize) -> MateDof {
        match self {
            Self::Screw | Self::Gear => MateDof::Rz,
            Self::Linear => MateDof::Tz,
            Self::RackAndPinion => {
                if index == 0 {
                    MateDof::Rz
                } else {
                    MateDof::Tz
                }
            }
        }
    }
}

/// The degree of freedom a relation couples on a mate.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum MateDof {
    /// Rotation about the mate's Z axis.
    Rz,
    /// Translation along the mate's Z axis.
    Tz,
}

impl MateDof {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rz => "Rz",
            Self::Tz => "Tz",
        }
    }
}

/// A mate taking part in a relation.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct RelationMate {
    /// Feature ID of the mate.
    pub feature_id: String,
    /// Degree of freedom to couple. Default: Rz for SCREW and GEAR, Tz for
    /// LINEAR, and Rz then Tz for `RACK_AND_PINION` (pinion first, rack
    /// second).
    #[serde(default)]
    pub dof: Option<MateDof>,
}

/// Input for `onshape_mate_relation`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct MateRelationInput {
    /// Document ID.
    pub did: String,
    /// Workspace ID.
    pub wid: String,
    /// Assembly element ID.
    pub eid: String,
    /// Relation name.
    pub name: String,
    /// Relation type.
    pub relation_type: RelationType,
    /// The mates to couple: one CYLINDRICAL mate for SCREW, two mates
    /// otherwise. Mates made in the UI or by `onshape_mate` work; relations
    /// need the mate's `secondaryAxisAlignment` parameter, which both write.
    pub mates: Vec<RelationMate>,
    /// Travel per revolution in millimetres, for SCREW (the thread lead,
    /// e.g. 1.27 for 1/4-20) and `RACK_AND_PINION`.
    #[serde(default)]
    pub distance_per_rev_mm: Option<f64>,
    /// Ratio for GEAR and LINEAR. Default 1.
    #[serde(default)]
    pub ratio: Option<f64>,
    /// Reverse the coupling direction.
    #[serde(default)]
    pub reverse: bool,
}

/// Inference types for placing a mate connector on an entity.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Inference {
    /// Centroid of a planar face.
    Centroid,
    /// Centre of a circular edge.
    Center,
    /// Midpoint of a line edge.
    MidPoint,
    /// Top point on a cylinder or cone axis.
    TopAxisPoint,
    /// Middle point on a cylinder or cone axis.
    MidAxisPoint,
    /// Bottom point on a cylinder or cone axis.
    BottomAxisPoint,
    /// A vertex.
    Point,
    /// Centre of a closed loop.
    LoopCenter,
    /// Corner behind a fillet or chamfer.
    VirtualSharp,
    /// The part's origin.
    PartOrigin,
}

impl Inference {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Centroid => "CENTROID",
            Self::Center => "CENTER",
            Self::MidPoint => "MID_POINT",
            Self::TopAxisPoint => "TOP_AXIS_POINT",
            Self::MidAxisPoint => "MID_AXIS_POINT",
            Self::BottomAxisPoint => "BOTTOM_AXIS_POINT",
            Self::Point => "POINT",
            Self::LoopCenter => "LOOP_CENTER",
            Self::VirtualSharp => "VIRTUAL_SHARP",
            Self::PartOrigin => "PART_ORIGIN",
        }
    }
}

/// Axis for a mate connector rotation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RotationAxis {
    /// About the connector X axis.
    AboutX,
    /// About the connector Y axis.
    AboutY,
    /// About the connector Z axis.
    #[default]
    AboutZ,
}

impl RotationAxis {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AboutX => "ABOUT_X",
            Self::AboutY => "ABOUT_Y",
            Self::AboutZ => "ABOUT_Z",
        }
    }
}

/// Input for `onshape_mate_connector`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct MateConnectorInput {
    /// Document ID.
    pub did: String,
    /// Workspace ID.
    pub wid: String,
    /// Assembly element ID.
    pub eid: String,
    /// Connector name.
    pub name: String,
    /// Occurrence path of the part instance, top level first.
    pub path: Vec<String>,
    /// Deterministic IDs of the entity in the part's Part Studio: a face,
    /// edge or vertex ID (`transientQueriesToStrings` in `FeatureScript` returns
    /// the same IDs). Empty only for `PART_ORIGIN`.
    #[serde(default)]
    pub entity_ids: Vec<String>,
    /// How to place the connector on the entity.
    pub inference: Inference,
    /// Flip the connector's primary (Z) axis.
    #[serde(default)]
    pub flip: bool,
    /// Offset `[x, y, z]` in millimetres, in the connector's own axes.
    #[serde(default)]
    pub offset_mm: Option<[f64; 3]>,
    /// Rotation in degrees about `rotation_axis`.
    #[serde(default)]
    pub rotation_deg: Option<f64>,
    /// Axis for `rotation_deg`. Default `ABOUT_Z`.
    #[serde(default)]
    pub rotation_axis: RotationAxis,
}

// ============================================================================
// Tool definitions
// ============================================================================

#[allow(clippy::expect_used)] // Schema generation for these plain structs cannot fail.
fn schema_object<T: JsonSchema>() -> Arc<Map<String, Value>> {
    let schema = schemars::schema_for!(T);
    let value = serde_json::to_value(schema).expect("input schema serialization should never fail");
    Arc::new(
        value
            .as_object()
            .cloned()
            .expect("input schema should be a JSON object"),
    )
}

/// Definitions of all tools in this module.
#[must_use]
pub fn tool_defs() -> Vec<Tool> {
    vec![
        Tool::new(
            TOOL_ASSEMBLY_READ,
            "Read an assembly in one call: instances, every occurrence with its path, fixed state \
             and position (mm), mates with the occurrences they join, other assembly features \
             (relations, groups, connectors), and Part Studio mate connectors on inserted parts. \
             Use it before mating, and to find which mate holds a part.",
            schema_object::<AssemblyReadInput>(),
        )
        .annotate(ToolAnnotations::new().read_only(true).destructive(false)),
        Tool::new(
            TOOL_MATE,
            "Add a mate between two existing mate connectors in one call: an assembly connector \
             feature, or a Part Studio connector on an inserted part (path + feature id). Types: \
             FASTENED, REVOLUTE, SLIDER, CYLINDRICAL, PIN_SLOT, PLANAR, BALL, PARALLEL. Frames \
             align in the same direction; set flip for face-to-face contact. Returns the feature \
             id and regeneration status.",
            schema_object::<MateInput>(),
        )
        .annotate(ToolAnnotations::new().read_only(false).destructive(false)),
        Tool::new(
            TOOL_MATE_RELATION,
            "Add a mate relation in one call: SCREW (one cylindrical mate, travel per revolution \
             in mm), GEAR or LINEAR (two mates, ratio), RACK_AND_PINION (pinion mate then rack \
             mate, travel per revolution in mm). Returns the feature id and regeneration status.",
            schema_object::<MateRelationInput>(),
        )
        .annotate(ToolAnnotations::new().read_only(false).destructive(false)),
        Tool::new(
            TOOL_MATE_CONNECTOR,
            "Add an assembly mate connector on a part entity in one call, placed by inference: \
             CENTER of a circular edge (holes), TOP/MID/BOTTOM_AXIS_POINT of a cylinder, \
             CENTROID of a planar face, POINT, MID_POINT, LOOP_CENTER, VIRTUAL_SHARP or \
             PART_ORIGIN. Optional flip, offset (mm, connector axes) and rotation. Prefer \
             Part Studio connectors for reused parts. Returns the feature id and status.",
            schema_object::<MateConnectorInput>(),
        )
        .annotate(ToolAnnotations::new().read_only(false).destructive(false)),
    ]
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch one of this module's tools. Returns `None` for other names.
#[must_use]
pub fn call(
    name: &str,
    arguments: Option<&Map<String, Value>>,
    spec: &OpenApiSpec,
) -> Option<ToolEffect> {
    let effect = match name {
        TOOL_ASSEMBLY_READ => call_assembly_read(arguments, spec),
        TOOL_MATE => call_mate(arguments, spec),
        TOOL_MATE_RELATION => call_mate_relation(arguments, spec),
        TOOL_MATE_CONNECTOR => call_mate_connector(arguments, spec),
        _ => return None,
    };
    Some(effect)
}

fn require_ids(ids: &[(&str, &str)]) -> Result<(), String> {
    for (name, value) in ids {
        if value.trim().is_empty() {
            return Err(format!("{name} must not be empty"));
        }
    }
    Ok(())
}

fn call_assembly_read(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
    let input: AssemblyReadInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };
    if !["w", "v", "m"].contains(&input.wvm.as_str()) {
        return tool_input_error(format!(
            "invalid wvm value \"{}\": expected \"w\", \"v\", or \"m\"",
            input.wvm
        ));
    }
    if let Err(message) = require_ids(&[
        ("did", &input.did),
        ("wvmid", &input.wvmid),
        ("eid", &input.eid),
    ]) {
        return tool_input_error(message);
    }
    let path_params = path_params(&input.did, &input.wvm, &input.wvmid, &input.eid);
    let query_params: HashMap<String, String> = [
        ("includeMateFeatures", "true".to_string()),
        (
            "includeMateConnectors",
            input.include_part_studio_connectors.to_string(),
        ),
        ("includeNonSolids", "false".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    match spec.build_request(
        "getAssemblyDefinition",
        &path_params,
        &query_params,
        &HeaderMap::new(),
        None,
    ) {
        Ok(request) => ToolEffect::ApiRequest {
            request,
            continuation: Continuation::FormatAssemblyDefinition,
        },
        Err(e) => tool_input_error(format!("failed to build assembly definition request: {e}")),
    }
}

fn call_mate(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
    let input: MateInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };
    if let Err(message) = require_ids(&[
        ("did", &input.did),
        ("wid", &input.wid),
        ("eid", &input.eid),
    ]) {
        return tool_input_error(message);
    }
    match build_mate_feature(&input.name, input.mate_type, &input.connectors, input.flip) {
        Ok(body) => add_feature_effect(spec, &input.did, &input.wid, &input.eid, body, "mate"),
        Err(message) => tool_input_error(message),
    }
}

fn call_mate_relation(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
    let input: MateRelationInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };
    if let Err(message) = require_ids(&[
        ("did", &input.did),
        ("wid", &input.wid),
        ("eid", &input.eid),
    ]) {
        return tool_input_error(message);
    }
    match build_relation_feature(&input) {
        Ok(body) => add_feature_effect(
            spec,
            &input.did,
            &input.wid,
            &input.eid,
            body,
            "mate relation",
        ),
        Err(message) => tool_input_error(message),
    }
}

fn call_mate_connector(arguments: Option<&Map<String, Value>>, spec: &OpenApiSpec) -> ToolEffect {
    let input: MateConnectorInput = match parse_arguments(arguments) {
        Ok(input) => input,
        Err(e) => return tool_input_error(e.message),
    };
    if let Err(message) = require_ids(&[
        ("did", &input.did),
        ("wid", &input.wid),
        ("eid", &input.eid),
    ]) {
        return tool_input_error(message);
    }
    match build_mate_connector_feature(&input) {
        Ok(body) => add_feature_effect(
            spec,
            &input.did,
            &input.wid,
            &input.eid,
            body,
            "mate connector",
        ),
        Err(message) => tool_input_error(message),
    }
}

fn path_params(did: &str, wvm: &str, wvmid: &str, eid: &str) -> HashMap<String, String> {
    [("did", did), ("wvm", wvm), ("wvmid", wvmid), ("eid", eid)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn add_feature_effect(
    spec: &OpenApiSpec,
    did: &str,
    wid: &str,
    eid: &str,
    body: Value,
    kind: &str,
) -> ToolEffect {
    match spec.build_request(
        "addFeature",
        &path_params(did, "w", wid, eid),
        &HashMap::new(),
        &HeaderMap::new(),
        Some(body),
    ) {
        Ok(request) => ToolEffect::ApiRequest {
            request,
            continuation: Continuation::FormatAssemblyFeature {
                kind: kind.to_string(),
            },
        },
        Err(e) => tool_input_error(format!("failed to build {kind} request: {e}")),
    }
}

// ============================================================================
// Payload builders (pure)
// ============================================================================

fn p_enum(parameter_id: &str, enum_name: &str, value: &str) -> Value {
    json!({
        "btType": "BTMParameterEnum-145",
        "namespace": "",
        "enumName": enum_name,
        "value": value,
        "parameterId": parameter_id,
    })
}

fn p_bool(parameter_id: &str, value: bool) -> Value {
    json!({ "btType": "BTMParameterBoolean-144", "value": value, "parameterId": parameter_id })
}

fn p_quantity(parameter_id: &str, expression: &str) -> Value {
    json!({
        "btType": "BTMParameterQuantity-147",
        "isInteger": false,
        "expression": expression,
        "parameterId": parameter_id,
    })
}

/// Format a number for an Onshape expression without exponent notation and
/// without losing precision (up to 12 decimals, trailing zeros trimmed).
fn number_expr(value: f64) -> String {
    let text = format!("{value:.12}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn connector_query(connector: &ConnectorRef) -> Result<Value, String> {
    match connector {
        ConnectorRef::AssemblyFeature { feature_id } => {
            if feature_id.trim().is_empty() {
                return Err("connector feature_id must not be empty".to_string());
            }
            Ok(json!({
                "btType": "BTMFeatureQueryWithOccurrence-157",
                "path": [],
                "featureId": feature_id,
                "queryData": "",
            }))
        }
        ConnectorRef::PartStudio { path, feature_id } => {
            if path.is_empty() || path.iter().any(|p| p.trim().is_empty()) {
                return Err(format!(
                    "Part Studio connector {feature_id} needs the instance's occurrence path"
                ));
            }
            if feature_id.trim().is_empty() {
                return Err("connector feature_id must not be empty".to_string());
            }
            Ok(json!({
                "btType": "BTMPartStudioMateConnectorQuery-1324",
                "path": path,
                "featureId": feature_id,
            }))
        }
    }
}

/// Build the `addFeature` body for a mate.
///
/// # Errors
///
/// Returns a message when the connector list is not exactly two valid
/// references or the name is empty.
pub fn build_mate_feature(
    name: &str,
    mate_type: MateType,
    connectors: &[ConnectorRef],
    flip: bool,
) -> Result<Value, String> {
    if name.trim().is_empty() {
        return Err("name must not be empty".to_string());
    }
    if connectors.len() != 2 {
        return Err(format!(
            "a mate joins exactly two connectors; got {}",
            connectors.len()
        ));
    }
    let queries = connectors
        .iter()
        .map(connector_query)
        .collect::<Result<Vec<_>, _>>()?;
    let mut parameters = vec![
        p_enum("mateType", "Mate type", mate_type.as_str()),
        json!({
            "btType": "BTMParameterQueryWithOccurrenceList-67",
            "queries": queries,
            "parameterId": "mateConnectorsQuery",
        }),
        // The UI always writes this; relations fail to solve without it.
        p_enum(
            "secondaryAxisAlignment",
            "Reorient secondary axis",
            "PLUS_X",
        ),
    ];
    if flip {
        parameters.push(p_bool("primaryAxisAlignment", true));
    }
    Ok(json!({
        "feature": {
            "btType": "BTMMate-64",
            "featureType": "mate",
            "name": name,
            "suppressed": false,
            "parameters": parameters,
        }
    }))
}

/// Build the `addFeature` body for a mate relation.
///
/// # Errors
///
/// Returns a message when the mate count, distance or ratio does not suit
/// the relation type.
pub fn build_relation_feature(input: &MateRelationInput) -> Result<Value, String> {
    let kind = input.relation_type;
    if input.name.trim().is_empty() {
        return Err("name must not be empty".to_string());
    }
    if input.mates.len() != kind.mate_count() {
        return Err(format!(
            "a {} relation takes exactly {} mate(s); got {}",
            kind.as_str(),
            kind.mate_count(),
            input.mates.len()
        ));
    }
    if input.mates.iter().any(|m| m.feature_id.trim().is_empty()) {
        return Err("mate feature_id must not be empty".to_string());
    }
    let size = if kind.uses_length() {
        match input.distance_per_rev_mm {
            Some(d) if d.is_finite() && d > 0.0 => {
                p_quantity("relationLength", &format!("{} mm", number_expr(d)))
            }
            _ => {
                return Err(format!(
                    "a {} relation needs a positive distance_per_rev_mm",
                    kind.as_str()
                ));
            }
        }
    } else {
        let ratio = input.ratio.unwrap_or(1.0);
        if !(ratio.is_finite() && ratio > 0.0) {
            return Err(format!(
                "a {} relation needs a positive ratio",
                kind.as_str()
            ));
        }
        p_quantity("relationRatio", &number_expr(ratio))
    };
    let queries: Vec<Value> = input
        .mates
        .iter()
        .enumerate()
        .map(|(i, mate)| {
            json!({
                "btType": "BTMFeatureQueryWithOccurrence-157",
                "path": [],
                "featureId": mate.feature_id,
                "queryData": mate.dof.unwrap_or_else(|| kind.default_dof(i)).as_str(),
            })
        })
        .collect();
    Ok(json!({
        "feature": {
            "btType": "BTMMateRelation-1412",
            "featureType": "mateRelation",
            "name": input.name,
            "suppressed": false,
            "parameters": [
                p_enum("relationType", "Relation type", kind.as_str()),
                {
                    "btType": "BTMParameterQueryWithOccurrenceList-67",
                    "queries": queries,
                    "parameterId": "matesQuery",
                },
                size,
                p_bool("reverseDirection", input.reverse),
            ],
        }
    }))
}

/// Build the `addFeature` body for an assembly mate connector.
///
/// # Errors
///
/// Returns a message when the path or entity list is missing or a number is
/// not finite.
pub fn build_mate_connector_feature(input: &MateConnectorInput) -> Result<Value, String> {
    if input.name.trim().is_empty() {
        return Err("name must not be empty".to_string());
    }
    if input.path.is_empty() || input.path.iter().any(|p| p.trim().is_empty()) {
        return Err("path must list the occurrence's instance IDs, top level first".to_string());
    }
    if input.inference != Inference::PartOrigin && input.entity_ids.is_empty() {
        return Err(format!(
            "inference {} needs entity_ids (a face, edge or vertex ID)",
            input.inference.as_str()
        ));
    }
    let mut parameters = vec![
        p_enum("originType", "Origin type", "ON_ENTITY"),
        json!({
            "btType": "BTMParameterQueryWithOccurrenceList-67",
            "queries": [{
                "btType": "BTMInferenceQueryWithOccurrence-1083",
                "inferenceType": input.inference.as_str(),
                "path": input.path,
                "deterministicIds": input.entity_ids,
            }],
            "parameterId": "originQuery",
        }),
    ];
    if input.flip {
        parameters.push(p_bool("flipPrimary", true));
    }
    let offset = input.offset_mm.unwrap_or([0.0; 3]);
    let rotation = input.rotation_deg.unwrap_or(0.0);
    if offset
        .iter()
        .chain(std::iter::once(&rotation))
        .any(|v| !v.is_finite())
    {
        return Err("offset_mm and rotation_deg must be finite numbers".to_string());
    }
    if offset.iter().any(|v| *v != 0.0) || rotation != 0.0 {
        parameters.push(p_bool("transform", true));
        for (axis, value) in ["translationX", "translationY", "translationZ"]
            .iter()
            .zip(offset)
        {
            parameters.push(p_quantity(axis, &format!("{} mm", number_expr(value))));
        }
        parameters.push(p_enum(
            "rotationType",
            "Rotation axis",
            input.rotation_axis.as_str(),
        ));
        parameters.push(p_quantity(
            "rotation",
            &format!("{} deg", number_expr(rotation)),
        ));
    }
    Ok(json!({
        "feature": {
            "btType": "BTMMateConnector-66",
            "featureType": "mateConnector",
            "name": input.name,
            "suppressed": false,
            "parameters": parameters,
        }
    }))
}

// ============================================================================
// Response formatting (pure)
// ============================================================================

fn json_result(structured: &Value, summary: String, is_error: bool) -> CallToolResult {
    let blocks = vec![
        ContentBlock::json(structured)
            .unwrap_or_else(|_| ContentBlock::text(structured.to_string())),
        ContentBlock::text(summary),
    ];
    if is_error {
        CallToolResult::error(blocks)
    } else {
        CallToolResult::success(blocks)
    }
}

/// Format the `addFeature` response for a mate, relation or connector.
#[must_use]
pub fn format_feature_response(kind: &str, body: &[u8]) -> CallToolResult {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return CallToolResult::error(vec![ContentBlock::text(format!(
            "{kind} request succeeded but the response was not JSON"
        ))]);
    };
    let feature = value.get("feature").unwrap_or(&Value::Null);
    let feature_id = feature
        .get("featureId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = feature.get("name").and_then(Value::as_str).unwrap_or("");
    let state = value.get("featureState").unwrap_or(&Value::Null);
    let status = state
        .get("featureStatus")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let structured = json!({
        "feature_id": feature_id,
        "name": name,
        "kind": kind,
        "status": status,
    });
    if status == "OK" {
        json_result(
            &structured,
            format!("Added {kind} \"{name}\" ({feature_id}): OK"),
            false,
        )
    } else {
        json_result(
            &structured,
            format!(
                "Added {kind} \"{name}\" ({feature_id}) but it regenerated with status {status}. \
                 The API call still counted. Read the feature or the assembly to find the cause, \
                 then delete or fix it before building on it."
            ),
            true,
        )
    }
}

struct Level<'a> {
    instances: &'a [Value],
}

/// Index of sub-assembly instance lists by (documentId, elementId,
/// documentMicroversion, fullConfiguration).
fn sub_assembly_index(definition: &Value) -> SubAssemblies<'_> {
    let mut index = HashMap::new();
    if let Some(subs) = definition.get("subAssemblies").and_then(Value::as_array) {
        for sub in subs {
            let key = assembly_key(sub);
            if let Some(instances) = sub.get("instances").and_then(Value::as_array) {
                index.insert(key, instances.as_slice());
            }
        }
    }
    index
}

fn str_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn assembly_key(value: &Value) -> (String, String, String, String) {
    (
        str_field(value, "documentId"),
        str_field(value, "elementId"),
        str_field(value, "documentMicroversion"),
        str_field(value, "fullConfiguration"),
    )
}

/// Resolve an occurrence path to instance names, level by level.
fn path_names(path: &[Value], root: &[Value], subs: &SubAssemblies<'_>) -> Vec<String> {
    let mut names = Vec::with_capacity(path.len());
    let mut level = Level { instances: root };
    for id in path {
        let id = id.as_str().unwrap_or("");
        let Some(instance) = level
            .instances
            .iter()
            .find(|i| i.get("id").and_then(Value::as_str) == Some(id))
        else {
            names.push(id.to_string());
            continue;
        };
        names.push(str_field(instance, "name"));
        if instance.get("type").and_then(Value::as_str) == Some("Assembly")
            && let Some(next) = subs.get(&assembly_key(instance))
        {
            level = Level { instances: next };
        }
    }
    names
}

fn round(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (value * factor).round() / factor
}

fn f64_at(values: &[Value], index: usize) -> f64 {
    values.get(index).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Summarise an occurrence transform: translation in mm, and the rotation
/// rows only when the occurrence is rotated.
fn transform_summary(transform: Option<&Value>) -> (Value, Option<Value>) {
    let Some(values) = transform.and_then(Value::as_array) else {
        return (Value::Null, None);
    };
    if values.len() < 12 {
        return (Value::Null, None);
    }
    let position = json!([
        round(f64_at(values, 3) * 1000.0, 4),
        round(f64_at(values, 7) * 1000.0, 4),
        round(f64_at(values, 11) * 1000.0, 4),
    ]);
    let rows: Vec<[f64; 3]> = [0, 4, 8]
        .iter()
        .map(|&start| {
            [
                round(f64_at(values, start), 6),
                round(f64_at(values, start + 1), 6),
                round(f64_at(values, start + 2), 6),
            ]
        })
        .collect();
    let identity = rows.iter().enumerate().all(|(r, row)| {
        row.iter()
            .enumerate()
            .all(|(c, v)| (v - f64::from(u8::from(r == c))).abs() < 1e-9)
    });
    let rotation = if identity { None } else { Some(json!(rows)) };
    (position, rotation)
}

fn point_mm(value: Option<&Value>) -> Value {
    value.and_then(Value::as_array).map_or(Value::Null, |a| {
        json!([
            round(f64_at(a, 0) * 1000.0, 4),
            round(f64_at(a, 1) * 1000.0, 4),
            round(f64_at(a, 2) * 1000.0, 4),
        ])
    })
}

type SubAssemblies<'a> = HashMap<(String, String, String, String), &'a [Value]>;

fn summarize_instances(root_instances: &[Value]) -> Vec<Value> {
    root_instances
        .iter()
        .map(|i| {
            let mut entry = json!({
                "id": str_field(i, "id"),
                "name": str_field(i, "name"),
                "type": str_field(i, "type"),
                "element_id": str_field(i, "elementId"),
                "suppressed": i.get("suppressed").and_then(Value::as_bool).unwrap_or(false),
            });
            if let Some(part_id) = i.get("partId").and_then(Value::as_str) {
                entry["part_id"] = json!(part_id);
            }
            entry
        })
        .collect()
}

fn summarize_occurrences(
    occurrences: &[Value],
    root_instances: &[Value],
    subs: &SubAssemblies<'_>,
) -> Vec<Value> {
    occurrences
        .iter()
        .map(|o| {
            let path = o
                .get("path")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let (position, rotation) = transform_summary(o.get("transform"));
            let mut entry = json!({
                "path": path,
                "name": path_names(&path, root_instances, subs).join(" / "),
                "fixed": o.get("fixed").and_then(Value::as_bool).unwrap_or(false),
                "position_mm": position,
            });
            if o.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
                entry["hidden"] = json!(true);
            }
            if let Some(rotation) = rotation {
                entry["rotation"] = rotation;
            }
            entry
        })
        .collect()
}

fn summarize_mate(
    feature: &Value,
    data: &Value,
    root_instances: &[Value],
    subs: &SubAssemblies<'_>,
) -> Value {
    let between: Vec<Value> = data
        .get("matedEntities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let path = e
                .get("matedOccurrence")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let cs = e.get("matedCS").or_else(|| e.get("mateConnectorCS"));
            json!({
                "path": path,
                "name": path_names(&path, root_instances, subs).join(" / "),
                "origin_mm": point_mm(cs.and_then(|c| c.get("origin"))),
            })
        })
        .collect();
    json!({
        "feature_id": str_field(feature, "id"),
        "name": str_field(data, "name"),
        "type": str_field(data, "mateType"),
        "suppressed": feature.get("suppressed").and_then(Value::as_bool).unwrap_or(false),
        "between": between,
    })
}

/// Split assembly features into mates and everything else.
fn summarize_features(
    features: &[Value],
    root_instances: &[Value],
    subs: &SubAssemblies<'_>,
) -> (Vec<Value>, Vec<Value>) {
    let mut mates = Vec::new();
    let mut other_features = Vec::new();
    for feature in features {
        let data = feature.get("featureData").unwrap_or(&Value::Null);
        let feature_type = str_field(feature, "featureType");
        if feature_type == "mate" {
            mates.push(summarize_mate(feature, data, root_instances, subs));
        } else {
            other_features.push(json!({
                "feature_id": str_field(feature, "id"),
                "name": str_field(data, "name"),
                "feature_type": feature_type,
                "suppressed": feature.get("suppressed").and_then(Value::as_bool).unwrap_or(false),
            }));
        }
    }
    (mates, other_features)
}

fn summarize_part_connectors(parts: &[Value]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|part| {
            let connectors = part.get("mateConnectors").and_then(Value::as_array)?;
            if connectors.is_empty() {
                return None;
            }
            let ids: Vec<Value> = connectors
                .iter()
                .filter_map(|c| c.get("featureId").cloned())
                .collect();
            Some(json!({
                "element_id": str_field(part, "elementId"),
                "part_id": str_field(part, "partId"),
                "connector_feature_ids": ids,
            }))
        })
        .collect()
}

fn array_field<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Summarise an assembly definition (with mate features and, optionally,
/// Part Studio mate connectors) into a compact JSON value.
#[must_use]
pub fn summarize_assembly(definition: &Value) -> Value {
    let root = definition.get("rootAssembly").unwrap_or(&Value::Null);
    let root_instances = array_field(root, "instances");
    let subs = sub_assembly_index(definition);
    let (mates, other_features) =
        summarize_features(array_field(root, "features"), root_instances, &subs);
    let part_studio_connectors = summarize_part_connectors(array_field(definition, "parts"));

    let mut summary = json!({
        "instances": summarize_instances(root_instances),
        "occurrences": summarize_occurrences(array_field(root, "occurrences"), root_instances, &subs),
        "mates": mates,
        "other_features": other_features,
    });
    if !part_studio_connectors.is_empty() {
        summary["part_studio_connectors"] = json!(part_studio_connectors);
    }
    summary
}

/// Format the `getAssemblyDefinition` response.
#[must_use]
pub fn format_assembly_definition(body: &[u8]) -> CallToolResult {
    let Ok(definition) = serde_json::from_slice::<Value>(body) else {
        return CallToolResult::error(vec![ContentBlock::text(
            "assembly definition response was not JSON",
        )]);
    };
    let summary = summarize_assembly(&definition);
    let count = |key: &str| {
        summary
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let fixed = summary
        .get("occurrences")
        .and_then(Value::as_array)
        .map_or(0, |o| {
            o.iter()
                .filter(|x| x.get("fixed").and_then(Value::as_bool).unwrap_or(false))
                .count()
        });
    let text = format!(
        "{} instances, {} occurrences ({} fixed), {} mates, {} other features",
        count("instances"),
        count("occurrences"),
        fixed,
        count("mates"),
        count("other_features"),
    );
    json_result(&summary, text, false)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn spec() -> OpenApiSpec {
        OpenApiSpec::from_json(
            r#"{
                "openapi": "3.0.1",
                "info": { "title": "Test", "version": "1.0" },
                "servers": [{ "url": "https://cad.onshape.com/api/v12" }],
                "paths": {
                    "/assemblies/d/{did}/{wvm}/{wvmid}/e/{eid}": {
                        "get": {
                            "operationId": "getAssemblyDefinition",
                            "tags": ["Assembly"],
                            "parameters": [
                                {"name": "did", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "wvm", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "wvmid", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "eid", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "includeMateFeatures", "in": "query", "schema": {"type": "boolean"}},
                                {"name": "includeMateConnectors", "in": "query", "schema": {"type": "boolean"}},
                                {"name": "includeNonSolids", "in": "query", "schema": {"type": "boolean"}}
                            ],
                            "responses": { "200": {} }
                        }
                    },
                    "/assemblies/d/{did}/{wvm}/{wvmid}/e/{eid}/features": {
                        "post": {
                            "operationId": "addFeature",
                            "tags": ["Assembly"],
                            "parameters": [
                                {"name": "did", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "wvm", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "wvmid", "in": "path", "required": true, "schema": {"type": "string"}},
                                {"name": "eid", "in": "path", "required": true, "schema": {"type": "string"}}
                            ],
                            "requestBody": {"content": {"application/json": {"schema": {"type": "object"}}}, "required": true},
                            "responses": { "200": {} }
                        }
                    }
                }
            }"#,
        )
        .expect("test spec parses")
    }

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object")
    }

    fn param<'a>(feature: &'a Value, id: &str) -> &'a Value {
        feature["feature"]["parameters"]
            .as_array()
            .expect("parameters")
            .iter()
            .find(|p| p["parameterId"] == id)
            .unwrap_or_else(|| panic!("parameter {id} missing"))
    }

    fn has_param(feature: &Value, id: &str) -> bool {
        feature["feature"]["parameters"]
            .as_array()
            .expect("parameters")
            .iter()
            .any(|p| p["parameterId"] == id)
    }

    // --- mates -------------------------------------------------------------

    #[test]
    fn mate_with_assembly_and_part_studio_connectors() {
        let body = build_mate_feature(
            "Clamp A - Torx Drive",
            MateType::Cylindrical,
            &[
                ConnectorRef::AssemblyFeature {
                    feature_id: "Mabc".into(),
                },
                ConnectorRef::PartStudio {
                    path: vec!["Msub".into(), "Mpart".into()],
                    feature_id: "FxY_0".into(),
                },
            ],
            false,
        )
        .expect("valid mate");
        assert_eq!(body["feature"]["btType"], "BTMMate-64");
        assert_eq!(body["feature"]["name"], "Clamp A - Torx Drive");
        assert_eq!(param(&body, "mateType")["value"], "CYLINDRICAL");
        let queries = &param(&body, "mateConnectorsQuery")["queries"];
        assert_eq!(queries[0]["btType"], "BTMFeatureQueryWithOccurrence-157");
        assert_eq!(queries[0]["featureId"], "Mabc");
        assert_eq!(queries[1]["btType"], "BTMPartStudioMateConnectorQuery-1324");
        assert_eq!(queries[1]["path"], json!(["Msub", "Mpart"]));
        assert_eq!(param(&body, "secondaryAxisAlignment")["value"], "PLUS_X");
        assert!(!has_param(&body, "primaryAxisAlignment"));
    }

    #[test]
    fn mate_flip_sets_primary_axis_alignment() {
        let body = build_mate_feature(
            "Seat",
            MateType::Fastened,
            &[
                ConnectorRef::AssemblyFeature {
                    feature_id: "A".into(),
                },
                ConnectorRef::AssemblyFeature {
                    feature_id: "B".into(),
                },
            ],
            true,
        )
        .expect("valid mate");
        assert_eq!(param(&body, "primaryAxisAlignment")["value"], true);
    }

    #[test]
    fn mate_rejects_wrong_connector_count_and_missing_path() {
        let one = [ConnectorRef::AssemblyFeature {
            feature_id: "A".into(),
        }];
        assert!(build_mate_feature("M", MateType::Revolute, &one, false).is_err());
        let no_path = [
            ConnectorRef::AssemblyFeature {
                feature_id: "A".into(),
            },
            ConnectorRef::PartStudio {
                path: vec![],
                feature_id: "F".into(),
            },
        ];
        let err =
            build_mate_feature("M", MateType::Revolute, &no_path, false).expect_err("needs path");
        assert!(err.contains("occurrence path"));
    }

    #[test]
    fn all_mate_types_serialize_to_onshape_names() {
        let names: Vec<&str> = [
            MateType::Fastened,
            MateType::Revolute,
            MateType::Slider,
            MateType::Cylindrical,
            MateType::PinSlot,
            MateType::Planar,
            MateType::Ball,
            MateType::Parallel,
        ]
        .iter()
        .map(|t| t.as_str())
        .collect();
        assert_eq!(
            names,
            [
                "FASTENED",
                "REVOLUTE",
                "SLIDER",
                "CYLINDRICAL",
                "PIN_SLOT",
                "PLANAR",
                "BALL",
                "PARALLEL"
            ]
        );
        let parsed: MateType = serde_json::from_value(json!("PIN_SLOT")).expect("parses");
        assert_eq!(parsed, MateType::PinSlot);
    }

    // --- relations ---------------------------------------------------------

    fn relation(kind: RelationType, mates: &[&str]) -> MateRelationInput {
        MateRelationInput {
            did: "d".into(),
            wid: "w".into(),
            eid: "e".into(),
            name: "R".into(),
            relation_type: kind,
            mates: mates
                .iter()
                .map(|id| RelationMate {
                    feature_id: (*id).into(),
                    dof: None,
                })
                .collect(),
            distance_per_rev_mm: None,
            ratio: None,
            reverse: false,
        }
    }

    #[test]
    fn screw_relation_uses_rz_and_length_in_mm() {
        let mut input = relation(RelationType::Screw, &["Mcyl"]);
        input.distance_per_rev_mm = Some(1.27);
        let body = build_relation_feature(&input).expect("valid screw");
        assert_eq!(body["feature"]["btType"], "BTMMateRelation-1412");
        assert_eq!(body["feature"]["featureType"], "mateRelation");
        assert_eq!(param(&body, "relationType")["value"], "SCREW");
        let queries = &param(&body, "matesQuery")["queries"];
        assert_eq!(queries[0]["featureId"], "Mcyl");
        assert_eq!(queries[0]["queryData"], "Rz");
        assert_eq!(param(&body, "relationLength")["expression"], "1.27 mm");
        assert_eq!(param(&body, "reverseDirection")["value"], false);
        assert!(!has_param(&body, "relationRatio"));
    }

    #[test]
    fn screw_relation_requires_distance() {
        let input = relation(RelationType::Screw, &["Mcyl"]);
        let err = build_relation_feature(&input).expect_err("needs distance");
        assert!(err.contains("distance_per_rev_mm"));
    }

    #[test]
    fn relation_mate_count_is_checked() {
        let input = relation(RelationType::Gear, &["M1"]);
        assert!(build_relation_feature(&input).is_err());
        let mut input = relation(RelationType::Screw, &["M1", "M2"]);
        input.distance_per_rev_mm = Some(1.0);
        assert!(build_relation_feature(&input).is_err());
    }

    #[test]
    fn rack_and_pinion_defaults_to_pinion_then_rack() {
        let mut input = relation(RelationType::RackAndPinion, &["Mpinion", "Mrack"]);
        input.distance_per_rev_mm = Some(31.4159);
        let body = build_relation_feature(&input).expect("valid rack");
        let queries = &param(&body, "matesQuery")["queries"];
        assert_eq!(queries[0]["queryData"], "Rz");
        assert_eq!(queries[1]["queryData"], "Tz");
    }

    #[test]
    fn gear_ratio_defaults_to_one_and_dof_can_be_overridden() {
        let mut input = relation(RelationType::Gear, &["M1", "M2"]);
        input.mates[1].dof = Some(MateDof::Tz);
        input.reverse = true;
        let body = build_relation_feature(&input).expect("valid gear");
        assert_eq!(param(&body, "relationRatio")["expression"], "1");
        assert_eq!(param(&body, "matesQuery")["queries"][1]["queryData"], "Tz");
        assert_eq!(param(&body, "reverseDirection")["value"], true);
    }

    // --- mate connectors ---------------------------------------------------

    fn connector(inference: Inference) -> MateConnectorInput {
        MateConnectorInput {
            did: "d".into(),
            wid: "w".into(),
            eid: "e".into(),
            name: "Hole axis".into(),
            path: vec!["Msub".into(), "Mclamp".into()],
            entity_ids: vec!["JHK".into()],
            inference,
            flip: false,
            offset_mm: None,
            rotation_deg: None,
            rotation_axis: RotationAxis::AboutZ,
        }
    }

    #[test]
    fn connector_on_hole_edge_has_no_transform() {
        let body = build_mate_connector_feature(&connector(Inference::Center)).expect("valid");
        assert_eq!(body["feature"]["btType"], "BTMMateConnector-66");
        let query = &param(&body, "originQuery")["queries"][0];
        assert_eq!(query["btType"], "BTMInferenceQueryWithOccurrence-1083");
        assert_eq!(query["inferenceType"], "CENTER");
        assert_eq!(query["path"], json!(["Msub", "Mclamp"]));
        assert_eq!(query["deterministicIds"], json!(["JHK"]));
        assert!(!has_param(&body, "transform"));
        assert!(!has_param(&body, "flipPrimary"));
    }

    #[test]
    fn connector_offset_and_rotation_use_mm_and_degrees() {
        let mut input = connector(Inference::Centroid);
        input.flip = true;
        input.offset_mm = Some([0.0, -8.192_77, 0.5]);
        input.rotation_deg = Some(90.0);
        input.rotation_axis = RotationAxis::AboutX;
        let body = build_mate_connector_feature(&input).expect("valid");
        assert_eq!(param(&body, "transform")["value"], true);
        assert_eq!(param(&body, "translationX")["expression"], "0 mm");
        assert_eq!(param(&body, "translationY")["expression"], "-8.19277 mm");
        assert_eq!(param(&body, "translationZ")["expression"], "0.5 mm");
        assert_eq!(param(&body, "rotationType")["value"], "ABOUT_X");
        assert_eq!(param(&body, "rotation")["expression"], "90 deg");
        assert_eq!(param(&body, "flipPrimary")["value"], true);
    }

    #[test]
    fn connector_requires_entity_except_part_origin() {
        let mut input = connector(Inference::Center);
        input.entity_ids.clear();
        assert!(build_mate_connector_feature(&input).is_err());
        let mut origin = connector(Inference::PartOrigin);
        origin.entity_ids.clear();
        assert!(build_mate_connector_feature(&origin).is_ok());
    }

    #[test]
    fn number_expr_keeps_precision_without_exponents() {
        assert_eq!(number_expr(1.27), "1.27");
        assert_eq!(number_expr(8.5), "8.5");
        assert_eq!(number_expr(0.000_000_5), "0.0000005");
        assert_eq!(number_expr(-0.0), "0");
        assert_eq!(number_expr(12.0), "12");
    }

    // --- dispatch ----------------------------------------------------------

    #[test]
    fn mate_tool_builds_add_feature_request() {
        let effect = call(
            TOOL_MATE,
            Some(&args(&json!({
                "did": "D", "wid": "W", "eid": "E", "name": "Pivot",
                "mate_type": "REVOLUTE",
                "connectors": [
                    {"kind": "assembly_feature", "feature_id": "M1"},
                    {"kind": "part_studio", "path": ["I1"], "feature_id": "F1"}
                ]
            }))),
            &spec(),
        )
        .expect("handled");
        match effect {
            ToolEffect::ApiRequest {
                request,
                continuation,
            } => {
                assert_eq!(request.method, http::Method::POST);
                assert!(
                    request.path.ends_with("/assemblies/d/D/w/W/e/E/features"),
                    "{}",
                    request.path
                );
                assert!(matches!(
                    continuation,
                    Continuation::FormatAssemblyFeature { .. }
                ));
            }
            other => panic!("expected ApiRequest, got {other:?}"),
        }
    }

    #[test]
    fn assembly_read_builds_definition_request_with_mate_features() {
        let effect = call(
            TOOL_ASSEMBLY_READ,
            Some(&args(&json!({"did": "D", "wvmid": "W", "eid": "E"}))),
            &spec(),
        )
        .expect("handled");
        match effect {
            ToolEffect::ApiRequest {
                request,
                continuation,
            } => {
                assert_eq!(request.method, http::Method::GET);
                assert!(
                    request.path.ends_with("/assemblies/d/D/w/W/e/E"),
                    "{}",
                    request.path
                );
                assert!(
                    request
                        .query_params
                        .contains(&("includeMateFeatures".to_string(), "true".to_string()))
                );
                assert!(matches!(
                    continuation,
                    Continuation::FormatAssemblyDefinition
                ));
            }
            other => panic!("expected ApiRequest, got {other:?}"),
        }
    }

    #[test]
    fn invalid_input_is_reported_without_a_request() {
        let effect = call(
            TOOL_MATE_RELATION,
            Some(&args(&json!({
                "did": "D", "wid": "W", "eid": "E", "name": "Screw",
                "relation_type": "SCREW", "mates": [{"feature_id": "M1"}]
            }))),
            &spec(),
        )
        .expect("handled");
        match effect {
            ToolEffect::Done(Ok(result)) => assert_eq!(result.is_error, Some(true)),
            other => panic!("expected input error, got {other:?}"),
        }
        assert!(call("onshape_api_call", None, &spec()).is_none());
    }

    // --- responses ---------------------------------------------------------

    #[test]
    fn feature_response_ok_and_error_states() {
        let ok = format_feature_response(
            "mate",
            br#"{"feature": {"featureId": "Mx", "name": "Pivot"}, "featureState": {"featureStatus": "OK"}}"#,
        );
        assert_ne!(ok.is_error, Some(true));
        let failed = format_feature_response(
            "mate",
            br#"{"feature": {"featureId": "My", "name": "Bad"}, "featureState": {"featureStatus": "ERROR"}}"#,
        );
        assert_eq!(failed.is_error, Some(true));
    }

    fn definition_fixture() -> Value {
        json!({
            "rootAssembly": {
                "documentId": "D", "elementId": "TOP", "documentMicroversion": "m0", "fullConfiguration": "default",
                "instances": [
                    {"id": "Mbollard", "name": "Bollard <1>", "type": "Part", "partId": "JHD",
                     "documentId": "D", "elementId": "PS1", "documentMicroversion": "m1", "suppressed": false},
                    {"id": "Mcam", "name": "Dual Camera Module <1>", "type": "Assembly",
                     "documentId": "D", "elementId": "CAM", "documentMicroversion": "m2",
                     "fullConfiguration": "default", "suppressed": false}
                ],
                "occurrences": [
                    {"path": ["Mbollard"], "fixed": true, "hidden": false,
                     "transform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]},
                    {"path": ["Mcam"], "fixed": false, "hidden": false,
                     "transform": [1,0,0,0, 0,1,0,0, 0,0,1,1.56606, 0,0,0,1]},
                    {"path": ["Mcam", "Mtube"], "fixed": false, "hidden": false,
                     "transform": [0,-1,0,0, 1,0,0,0, 0,0,1,1.56606, 0,0,0,1]}
                ],
                "features": [
                    {"id": "Mmate1", "featureType": "mate", "suppressed": false,
                     "featureData": {"name": "Camera to Coupler", "mateType": "FASTENED",
                        "matedEntities": [
                            {"matedOccurrence": ["Mcam", "Mtube"], "matedCS": {"origin": [0, 0, 1.56606]}},
                            {"matedOccurrence": ["Mbollard"], "matedCS": {"origin": [0, 0, 1.53416]}}
                        ]}},
                    {"id": "Mrel1", "featureType": "mateRelation", "suppressed": false,
                     "featureData": {"name": "Screw 1"}}
                ]
            },
            "subAssemblies": [
                {"documentId": "D", "elementId": "CAM", "documentMicroversion": "m2", "fullConfiguration": "default",
                 "instances": [
                     {"id": "Mtube", "name": "Camera Module Tube Section <1>", "type": "Part", "partId": "JHD"}
                 ]}
            ],
            "parts": [
                {"elementId": "PS1", "partId": "JHD", "mateConnectors": [{"featureId": "Ftop"}]},
                {"elementId": "PS2", "partId": "JKD", "mateConnectors": []}
            ]
        })
    }

    #[test]
    fn summary_resolves_nested_names_positions_and_mates() {
        let summary = summarize_assembly(&definition_fixture());
        let occurrences = summary["occurrences"].as_array().expect("occurrences");
        assert_eq!(occurrences.len(), 3);
        assert_eq!(occurrences[0]["fixed"], true);
        assert_eq!(occurrences[0]["position_mm"], json!([0.0, 0.0, 0.0]));
        assert!(occurrences[0].get("rotation").is_none());
        assert_eq!(
            occurrences[2]["name"],
            "Dual Camera Module <1> / Camera Module Tube Section <1>"
        );
        assert_eq!(occurrences[2]["position_mm"], json!([0.0, 0.0, 1566.06]));
        assert!(occurrences[2].get("rotation").is_some());

        let mates = summary["mates"].as_array().expect("mates");
        assert_eq!(mates.len(), 1);
        assert_eq!(mates[0]["type"], "FASTENED");
        assert_eq!(
            mates[0]["between"][0]["name"],
            "Dual Camera Module <1> / Camera Module Tube Section <1>"
        );
        assert_eq!(
            mates[0]["between"][1]["origin_mm"],
            json!([0.0, 0.0, 1534.16])
        );

        assert_eq!(summary["other_features"][0]["feature_type"], "mateRelation");
        let connectors = summary["part_studio_connectors"]
            .as_array()
            .expect("connectors");
        assert_eq!(connectors.len(), 1);
        assert_eq!(connectors[0]["connector_feature_ids"], json!(["Ftop"]));
    }

    #[test]
    fn definition_formatting_reports_counts() {
        let body = serde_json::to_vec(&definition_fixture()).expect("serializes");
        let result = format_assembly_definition(&body);
        assert_ne!(result.is_error, Some(true));
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("3 occurrences (1 fixed), 1 mates, 1 other features"),
            "{text}"
        );
    }

    #[test]
    fn tool_defs_cover_every_name() {
        let defs = tool_defs();
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(names, TOOL_NAMES);
    }
}
