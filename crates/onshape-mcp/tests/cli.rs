//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Helper for MCP protocol communication in tests.
struct McpTestClient {
    child: Child,
    stdin: std::process::ChildStdin,
    response_rx: mpsc::Receiver<std::io::Result<String>>,
    next_id: i64,
}

impl McpTestClient {
    fn spawn() -> Self {
        let binary_path = find_binary();
        let mut child = Command::new(&binary_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn binary at {}: {e}", binary_path.display()));

        let stdin = child.stdin.take().expect("failed to open stdin");
        let stdout = child.stdout.take().expect("failed to open stdout");

        // Spawn reader thread
        let (tx, response_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });

        Self {
            child,
            stdin,
            response_rx,
            next_id: 1,
        }
    }

    fn send_request(&mut self, method: &str, params: &serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let request_str = serde_json::to_string(&request).unwrap();
        writeln!(self.stdin, "{request_str}").expect("failed to write request");
        self.stdin.flush().expect("failed to flush stdin");

        let response_line = self
            .response_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timeout waiting for response")
            .expect("failed to read response");

        let response: serde_json::Value =
            serde_json::from_str(&response_line).expect("failed to parse JSON response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);

        response
    }

    fn send_notification(&mut self, method: &str) {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method
        });
        let notification_str = serde_json::to_string(&notification).unwrap();
        writeln!(self.stdin, "{notification_str}").expect("failed to write notification");
        self.stdin.flush().expect("failed to flush stdin");
    }

    fn initialize(&mut self) -> serde_json::Value {
        let response = self.send_request(
            "initialize",
            &serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "0.0.1"
                }
            }),
        );
        self.send_notification("notifications/initialized");
        response
    }

    fn shutdown(mut self) {
        drop(self.stdin);

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().expect("failed to wait for child") {
                assert!(status.success(), "process exited with error: {status}");
                return;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                panic!("timed out waiting for MCP server to exit");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

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

#[test]
fn tools_list_includes_auth_status_tool() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request("tools/list", &serde_json::json!({}));

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    let auth_status_tool = tools
        .iter()
        .find(|t| t["name"] == "onshape_mcp_auth_status")
        .expect("onshape_mcp_auth_status tool should be listed");

    assert!(
        auth_status_tool["description"]
            .as_str()
            .is_some_and(|d| d.contains("authentication status")),
        "tool should have a description mentioning authentication status"
    );

    client.shutdown();
}

#[test]
fn call_auth_status_tool_returns_not_configured() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_mcp_auth_status",
            "arguments": {}
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");

    // The tool returns JSON content
    let first_content = &content[0];
    assert_eq!(first_content["type"], "text");

    // Parse the JSON text content
    let text = first_content["text"]
        .as_str()
        .expect("text should be a string");
    let auth_result: serde_json::Value =
        serde_json::from_str(text).expect("text should be valid JSON");

    assert_eq!(auth_result["status"], "not_configured");
    assert!(auth_result["last_check"].is_null());
    assert_eq!(auth_result["message"], "No credentials configured");

    client.shutdown();
}
