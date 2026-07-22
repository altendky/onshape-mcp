# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added request-header support to `onshape_api_call`, including explicit
  `Accept` headers and required OpenAPI header parameters
- Added binary API responses with base64-encoded body metadata
- Added public sans-I/O Rust helpers for configuration, translations, external
  data downloads, and Part Studio or Assembly glTF/STEP exports
- Added the standalone `onshape-openapi` crate for OpenAPI search, explanation,
  and request building

### Changed

- Direct OAuth with a user-owned Onshape application is now the default; proxy
  login requires an explicit self-hosted proxy URL, and this project no longer
  offers public hosted MCP or OAuth proxy services
- OAuth and configuration storage now honors absolute XDG paths and consistently
  uses `%LOCALAPPDATA%` for Windows token storage
- Public Rust HTTP boundaries now use `http` crate types and preserve response
  headers and raw bytes
- Upgraded to rmcp 2.2 and the MCP 2025-11-25 model API; public Rust resource
  and content types now use `Resource` and `ContentBlock`
- Updated the embedded Onshape OpenAPI specification and FeatureScript error
  descriptions, including verified 3MF translation parameters

### Fixed

- Made OAuth token writes atomic and serialized, preventing refresh, login, and
  file-watcher races; incomplete token responses are now rejected
- Fixed dynamic calls that omitted OpenAPI path-level parameters
- Fixed parameterized JSON and binary content-type handling and binary endpoint
  content negotiation
- Updated `rustls-webpki` to address RustSec advisories

### Migration Notes

- Replace `onshape-mcp auth login --direct` with `onshape-mcp auth login`
- For proxy login, pass an explicit self-hosted URL with
  `onshape-mcp auth login --proxy-url https://your-proxy.example`; remote proxy
  URLs must use HTTPS
- Calls to `onshape_auth_login` now default to direct mode and therefore require
  nonblank `client_id` and `client_secret` values; proxy calls must explicitly
  set `mode` to `"proxy"` and provide a nonblank self-hosted `proxy_url`
- OpenCode now lists direct OAuth first and prompts for a user-owned client ID
  and secret; its proxy option requires an explicit self-hosted URL and has no
  project-provided default
- Remove configurations that reference the former hosted MCP server or public
  OAuth proxy
- Windows users may need to authenticate again because tokens are now read from
  `%LOCALAPPDATA%\onshape-mcp\tokens.json` rather than `%APPDATA%`
- Rust consumers of `OAuthTokenData` must remove the former `client_id`,
  `client_secret`, and `proxy_url` fields from struct literals and field access;
  persisted MCP token files retain those metadata keys, but there is no public
  replacement metadata type
- Import `default_data_dir` and `default_token_file_path` from
  `onshape_mcp_io::oauth` instead of `onshape_client_core::oauth`
- `OpenApiSpec::build_request` moved to `onshape_openapi` and now takes an
  `&http::HeaderMap` argument between `query_params` and `body`; callers without
  headers can pass `&HeaderMap::new()`
- `IoResult::ApiResponse` now includes `headers: &[(String, String)]`, and its
  `body` changed from `&str` to `&[u8]`; constructors and exhaustive patterns
  must add `headers`, and text bodies must be converted to bytes
- `process_api_response` changed from `(status, body: &str)` to
  `(status, headers: &[(String, String)], body: &[u8])`; pass `&[]` when no
  headers are available and preserve raw response bytes
- Rust consumers should use `http::Method`, `http::StatusCode`, and
  `http::HeaderMap`, handle `ApiRequest.headers`, and read response bodies with
  `ResponseBody::as_bytes()` or `ResponseBody::text()`
- Import OpenAPI helpers from `onshape_openapi` instead of
  `onshape_mcp_core::openapi`, and update rmcp resource/content consumers to
  `Resource` and `ContentBlock`
