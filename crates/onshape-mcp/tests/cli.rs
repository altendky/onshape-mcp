//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Find the binary path, handling both normal cargo test and nextest archive contexts.
///
/// # Panics
///
/// Panics if the binary does not exist at the resolved path.
fn find_binary() -> PathBuf {
    // Runtime: nextest sets this correctly even for archives
    if let Some(path) = std::env::var_os("NEXTEST_BIN_EXE_onshape-mcp")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }

    // Fallback for regular cargo test.
    // Note: This is a compile-time constant; if the binary doesn't exist
    // at runtime (e.g., deleted or relocated), we need to fail with a
    // clear error message.
    let path = PathBuf::from(env!("CARGO_BIN_EXE_onshape-mcp"));
    assert!(
        path.exists(),
        "Binary not found at {}. \
        If running nextest archives, ensure NEXTEST_BIN_EXE_onshape-mcp is set.",
        path.display()
    );
    path
}

#[test]
fn mcp_initialization_returns_server_info() {
    let binary_path = find_binary();

    // Spawn the MCP server with stdin/stdout pipes
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn binary at {binary_path:?}: {e}"));

    let mut stdin = child.stdin.take().expect("failed to open stdin");
    let stdout = child.stdout.take().expect("failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Send MCP initialize request (JSON-RPC over stdio)
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "0.0.1"
            }
        }
    });

    let request_str = serde_json::to_string(&init_request).unwrap();
    writeln!(stdin, "{request_str}").expect("failed to write to stdin");
    stdin.flush().expect("failed to flush stdin");

    // Read the response (one line of JSON) with timeout
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut response_line = String::new();
        let res = reader.read_line(&mut response_line).map(|_| response_line);
        let _ = tx.send(res);
    });
    let response_line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("timeout waiting for response")
        .expect("failed to read response");

    // Parse and verify the response
    let response: serde_json::Value =
        serde_json::from_str(&response_line).expect("failed to parse JSON response");

    // Verify JSON-RPC structure
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["error"].is_null(), "unexpected error: {response}");

    // Verify server info in result
    let result = &response["result"];
    assert_eq!(
        result["serverInfo"]["name"], "onshape-mcp",
        "unexpected server name"
    );
    assert!(
        result["serverInfo"]["version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "server version should be non-empty"
    );

    // Verify tools capability is enabled
    assert!(
        result["capabilities"]["tools"].is_object(),
        "tools capability should be enabled"
    );

    // Send the initialized notification (required by MCP protocol)
    let initialized_notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let notification_str = serde_json::to_string(&initialized_notification).unwrap();
    writeln!(stdin, "{notification_str}").expect("failed to write initialized notification");
    stdin.flush().expect("failed to flush stdin");

    // Close stdin to signal shutdown
    drop(stdin);

    // Wait for the process to exit with timeout
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("failed to wait for child") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("timed out waiting for MCP server to exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "process exited with error: {status}");
}
