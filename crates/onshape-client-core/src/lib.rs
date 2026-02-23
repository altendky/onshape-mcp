//! Pure Onshape API logic and types.
//!
//! This crate contains sans-IO types and logic for interacting with the Onshape REST API.
//! No async runtime, HTTP client, or network access — all I/O is handled by `onshape-client-io`.

pub mod auth;
pub mod oauth;
pub mod request;
