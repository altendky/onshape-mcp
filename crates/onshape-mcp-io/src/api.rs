//! Execute generic API effects using a host-supplied request executor.
//!
//! Authentication, refresh, and host state updates belong to the executor.
//! This runner reads permitted files and feeds I/O results back to the pure engine.

#[cfg(test)]
mod tests;

use std::future::Future;

use onshape_mcp_core::tools::api::{self, Effect, FileRead, FileReadResult, IoResult, Policy};
use onshape_openapi::request::ApiRequest;
use rmcp::{
    ErrorData,
    model::{CallToolResult, ContentBlock},
};

/// Raw HTTP response data passed back to the pure API engine.
#[derive(Debug)]
pub struct Response {
    /// HTTP status code, including non-success statuses.
    pub status: u16,
    /// Response headers as name/value pairs.
    pub headers: Vec<(String, String)>,
    /// Unmodified response body bytes.
    pub body: Vec<u8>,
}

/// A response to format, or a host result that completes the tool immediately.
pub enum RequestOutcome {
    /// Feed an HTTP response to the pending continuation.
    Response(Response),
    /// Return a host diagnostic, such as missing credentials, without resuming.
    ToolResult(CallToolResult),
}

/// Host execution of neutral requests, including any authentication and state updates.
pub trait RequestExecutor: Send {
    /// Execute a request or return a host diagnostic that completes the tool.
    ///
    /// # Errors
    ///
    /// Return an MCP error for transport or infrastructure failures. HTTP error
    /// statuses should be returned as responses for the pure engine to format.
    fn execute(
        &mut self,
        request: ApiRequest,
    ) -> impl Future<Output = Result<RequestOutcome, ErrorData>> + Send;
}

/// Whether the host permits local file reads for this invocation.
pub enum FileReadPolicy<'a> {
    /// Read the files requested by the pure engine.
    Allow,
    /// Complete with the host's tool-error diagnostic before touching the filesystem.
    Deny(&'a str),
}

/// Run API effects to completion, resuming the pure engine after each I/O operation.
///
/// Host policy is supplied separately; continuations remain plain data.
///
/// # Errors
///
/// Propagates protocol errors from the engine and request executor. File read
/// failures and denied access are returned as tool-level errors.
pub async fn run(
    initial_effect: Effect,
    policy: &Policy,
    executor: &mut impl RequestExecutor,
    file_reads: FileReadPolicy<'_>,
) -> Result<CallToolResult, ErrorData> {
    let mut current = initial_effect;
    loop {
        current = match current {
            Effect::Done(result) => return result,
            Effect::ApiRequest {
                request,
                continuation,
            } => match executor.execute(request).await? {
                RequestOutcome::Response(response) => api::resume(
                    continuation,
                    IoResult::ApiResponse {
                        status: response.status,
                        headers: &response.headers,
                        body: &response.body,
                    },
                    policy,
                ),
                RequestOutcome::ToolResult(result) => return Ok(result),
            },
            Effect::ReadFiles {
                reads,
                continuation,
            } => {
                let results = match read_files(&reads, &file_reads).await {
                    Ok(results) => results,
                    Err(result) => return Ok(result),
                };
                api::resume(continuation, IoResult::FileReadResults(&results), policy)
            }
        };
    }
}

/// Apply host permission, then return one read result per requested file.
pub(crate) async fn read_files(
    reads: &[FileRead],
    policy: &FileReadPolicy<'_>,
) -> Result<Vec<FileReadResult>, CallToolResult> {
    if let FileReadPolicy::Deny(message) = policy {
        return Err(CallToolResult::error(vec![ContentBlock::text(*message)]));
    }
    let mut results = Vec::with_capacity(reads.len());
    for read in reads {
        match tokio::fs::read(&read.path).await {
            Ok(data) => results.push(FileReadResult::Success {
                path: read.path.clone(),
                data,
            }),
            Err(error) => results.push(FileReadResult::Error {
                path: read.path.clone(),
                message: format!("failed to read file: {error}"),
            }),
        }
    }
    Ok(results)
}
