//! Pure request and response helpers for Onshape API endpoints.
//!
//! These helpers build [`crate::request::ApiRequest`] values and parse response
//! data without performing network, filesystem, clock, storage, or runtime I/O.

mod configuration;
mod exports;
mod external_data;
mod shared;
mod translations;

pub use configuration::{EncodeConfigurationMapOptions, GetConfigurationOptions};
pub use configuration::{encode_configuration_map, get_configuration};
pub use exports::{
    ExportAdvancedParams, ExportMeshParams, GltfExportRequestBody, StepExportRequestBody,
};
pub use exports::{
    create_assembly_export_gltf, create_assembly_export_step, create_part_studio_export_gltf,
    create_part_studio_export_step,
};
pub use external_data::download_external_data;
pub use shared::{
    ElementRef, Error, WorkspaceVersion, WorkspaceVersionMicroversion, WvmElementRef,
};
pub use translations::{
    MeshTranslationRequestBody, TranslationMeshParams, TranslationRequestInfo,
    TranslationRequestState,
};
pub use translations::{
    create_assembly_translation, create_part_studio_translation, get_all_translator_formats,
    get_translation, parse_translation_request_info,
};
