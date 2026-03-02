//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use onshape_mcp_io::config::ENV_PREFIX;

/// Helper for MCP protocol communication in tests.
struct McpTestClient {
    child: Child,
    stdin: std::process::ChildStdin,
    response_rx: mpsc::Receiver<std::io::Result<String>>,
    next_id: i64,
    /// Temp directory for XDG isolation — kept alive for the lifetime of the client.
    _isolation_dir: tempfile::TempDir,
}

impl McpTestClient {
    fn spawn() -> Self {
        Self::spawn_with_env_and_args(&HashMap::new(), &[])
    }

    fn spawn_with_env(env: &HashMap<&str, &str>) -> Self {
        Self::spawn_with_env_and_args(env, &[])
    }

    fn spawn_with_args(args: &[&str]) -> Self {
        Self::spawn_with_env_and_args(&HashMap::new(), args)
    }

    fn spawn_with_env_and_args(env: &HashMap<&str, &str>, args: &[&str]) -> Self {
        let isolation_dir = tempfile::tempdir().expect("should create isolation temp dir");
        let mut cmd = isolated_command(isolation_dir.path());
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env {
            cmd.env(key, value);
        }
        for arg in args {
            cmd.arg(arg);
        }
        let mut child = cmd.spawn().unwrap_or_else(|e| {
            panic!("failed to spawn binary at {}: {e}", find_binary().display())
        });

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
            _isolation_dir: isolation_dir,
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

        // Loop until we find a response with matching id, handling notifications
        // and out-of-order responses gracefully
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timeout waiting for response with id {id}"
            );

            let response_line = match self.response_rx.recv_timeout(remaining) {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => panic!("failed to read response: {e}"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("timeout waiting for response with id {id}");
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("reader thread disconnected while waiting for response");
                }
            };

            // Continue on parse errors (malformed messages)
            let response: serde_json::Value = match serde_json::from_str(&response_line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Check if this is a response with matching id
            // Messages without an "id" field are notifications - skip them
            // Messages with non-matching id are responses to other requests - skip them
            if let Some(response_id) = response.get("id")
                && response_id == id
            {
                assert_eq!(response["jsonrpc"], "2.0");
                return response;
            }
            // Not our response (notification or different id), keep waiting
        }
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

/// Create a [`Command`] for the server binary with environment isolation.
///
/// Sets up XDG overrides and clears `ONSHAPE_MCP_*` env vars so the spawned
/// process cannot pick up the developer's local config/token files or env-based
/// credentials.
///
/// The caller must keep the `isolation_dir` alive for the lifetime of the process.
fn isolated_command(isolation_dir: &std::path::Path) -> Command {
    let binary_path = find_binary();
    let mut cmd = Command::new(&binary_path);

    // Clear all ONSHAPE_MCP_ env vars to isolate tests
    for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with(ENV_PREFIX)) {
        cmd.env_remove(&key);
    }

    // Override XDG directories so dirs::data_dir() and dirs::config_dir()
    // resolve to empty temp directories instead of the user's real ones.
    cmd.env("XDG_DATA_HOME", isolation_dir.join("data"));
    cmd.env("XDG_CONFIG_HOME", isolation_dir.join("config"));
    // Belt-and-suspenders: override HOME for platforms where dirs uses it
    // as a fallback (e.g., if XDG vars are somehow not respected).
    cmd.env("HOME", isolation_dir.join("home"));

    cmd
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
    let mut client = McpTestClient::spawn();
    let response = client.initialize();

    // Verify JSON-RPC structure
    assert_eq!(response["jsonrpc"], "2.0");
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

    client.shutdown();
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

// ============================================================================
// Auth Configuration Integration Tests
// ============================================================================

/// Helper to call `auth_status` and extract the parsed JSON result.
fn call_auth_status(client: &mut McpTestClient) -> serde_json::Value {
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

    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");
    serde_json::from_str(text).expect("text should be valid JSON")
}

#[test]
fn auth_status_not_validated_with_both_env_vars() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__ACCESS_KEY", "test-access-key");
    env.insert("ONSHAPE_MCP_AUTH__SECRET_KEY", "test-secret-key");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_validated");
    assert!(
        auth_result["message"]
            .as_str()
            .is_some_and(|m| m.contains("not yet validated"))
    );

    client.shutdown();
}

#[test]
fn auth_status_partial_with_only_access_key_env_var() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__ACCESS_KEY", "test-access-key");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_configured");
    assert!(
        auth_result["message"]
            .as_str()
            .is_some_and(|m| m.contains("secret_key")),
        "expected message to contain 'secret_key', got: {:?}",
        auth_result["message"]
    );

    client.shutdown();
}

#[test]
fn auth_status_partial_with_only_secret_key_env_var() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__SECRET_KEY", "test-secret-key");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_configured");
    assert!(
        auth_result["message"]
            .as_str()
            .is_some_and(|m| m.contains("access_key"))
    );

    client.shutdown();
}

#[test]
fn auth_status_not_validated_with_cli_flags() {
    let mut client = McpTestClient::spawn_with_args(&[
        "--access-key",
        "cli-access-key",
        "--secret-key",
        "cli-secret-key",
    ]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_validated");

    client.shutdown();
}

#[test]
fn auth_status_not_validated_with_config_file() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        "[auth]\naccess_key = \"file-ak\"\nsecret_key = \"file-sk\"\n",
    )
    .expect("should write config file");

    // Set secure permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("should set permissions");
    }

    let config_arg = format!("--config={}", config_path.display());
    let mut client = McpTestClient::spawn_with_args(&[&config_arg]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_validated");

    client.shutdown();
}

#[cfg(unix)]
#[test]
fn server_rejects_insecure_config_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        "[auth]\naccess_key = \"file-ak\"\nsecret_key = \"file-sk\"\n",
    )
    .expect("should write config file");

    // Set world-readable permissions (insecure)
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644))
        .expect("should set permissions");

    let config_arg = format!("--config={}", config_path.display());
    let isolation_dir = tempfile::tempdir().expect("should create isolation temp dir");
    let output = isolated_command(isolation_dir.path())
        .arg(&config_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should run binary");

    // The server should fail to start due to insecure permissions
    assert!(
        !output.status.success(),
        "server should reject insecure config file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The error may be rendered via Display (with octal mode) or Debug (with struct fields).
    // Both should reference the permission issue.
    assert!(
        stderr.contains("insecure permissions")
            || stderr.contains("InsecurePermissions")
            || stderr.contains("0644")
            || stderr.contains("0600"),
        "error should mention permissions, got: {stderr}"
    );
}

// ============================================================================
// Auth Method Configuration Tests
// ============================================================================

#[test]
fn auto_method_resolves_to_basic_with_api_keys() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__ACCESS_KEY", "test-access-key");
    env.insert("ONSHAPE_MCP_AUTH__SECRET_KEY", "test-secret-key");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(
        auth_result["auth_method"], "basic",
        "auto method should resolve to basic when API keys are provided"
    );

    client.shutdown();
}

#[test]
fn auth_method_basic_via_env_var() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__ACCESS_KEY", "test-access-key");
    env.insert("ONSHAPE_MCP_AUTH__SECRET_KEY", "test-secret-key");
    env.insert("ONSHAPE_MCP_AUTH__METHOD", "basic");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "basic");

    client.shutdown();
}

#[test]
fn auth_method_basic_via_cli_flag() {
    let mut client = McpTestClient::spawn_with_args(&[
        "--access-key",
        "cli-access-key",
        "--secret-key",
        "cli-secret-key",
        "--auth-method",
        "basic",
    ]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "basic");

    client.shutdown();
}

#[test]
fn auth_method_basic_via_config_file() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        "[auth]\naccess_key = \"file-ak\"\nsecret_key = \"file-sk\"\nmethod = \"basic\"\n",
    )
    .expect("should write config file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("should set permissions");
    }

    let config_arg = format!("--config={}", config_path.display());
    let mut client = McpTestClient::spawn_with_args(&[&config_arg]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "basic");

    client.shutdown();
}

// ============================================================================
// Invalid Auth Method Tests
// ============================================================================

#[test]
fn server_rejects_invalid_auth_method_via_cli_flag() {
    let isolation_dir = tempfile::tempdir().expect("should create isolation temp dir");
    let output = isolated_command(isolation_dir.path())
        .args([
            "--access-key",
            "ak",
            "--secret-key",
            "sk",
            "--auth-method",
            "nonsense",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should run binary");

    assert!(
        !output.status.success(),
        "server should reject invalid auth method"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown variant")
            || stderr.contains("nonsense")
            || (stderr.contains("auth") && stderr.contains("method")),
        "error should mention the invalid auth method, got: {stderr}"
    );
}

#[test]
fn server_rejects_invalid_auth_method_via_env_var() {
    let isolation_dir = tempfile::tempdir().expect("should create isolation temp dir");
    let output = isolated_command(isolation_dir.path())
        .env("ONSHAPE_MCP_AUTH__ACCESS_KEY", "ak")
        .env("ONSHAPE_MCP_AUTH__SECRET_KEY", "sk")
        .env("ONSHAPE_MCP_AUTH__METHOD", "nonsense")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should run binary");

    assert!(
        !output.status.success(),
        "server should reject invalid auth method from env var"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown variant")
            || stderr.contains("nonsense")
            || (stderr.contains("auth") && stderr.contains("method")),
        "error should mention the invalid auth method, got: {stderr}"
    );
}

#[test]
fn server_rejects_invalid_auth_method_via_config_file() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        "[auth]\naccess_key = \"file-ak\"\nsecret_key = \"file-sk\"\nmethod = \"nonsense\"\n",
    )
    .expect("should write config file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("should set permissions");
    }

    let config_arg = format!("--config={}", config_path.display());
    let isolation_dir = tempfile::tempdir().expect("should create isolation temp dir");
    let output = isolated_command(isolation_dir.path())
        .arg(&config_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should run binary");

    assert!(
        !output.status.success(),
        "server should reject invalid auth method from config file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown variant")
            || stderr.contains("nonsense")
            || (stderr.contains("auth") && stderr.contains("method")),
        "error should mention the invalid auth method, got: {stderr}"
    );
}

// ============================================================================
// Config Override Tests
// ============================================================================

#[test]
fn cli_flags_override_env_vars() {
    // Set env vars with one set of keys, CLI flags with another
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__ACCESS_KEY", "env-access-key");
    // Intentionally only set access_key via env (partial)
    // But provide both via CLI — CLI should win

    let mut client = McpTestClient::spawn_with_env_and_args(
        &env,
        &[
            "--access-key",
            "cli-access-key",
            "--secret-key",
            "cli-secret-key",
        ],
    );
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    // CLI provides both keys, so status should be not_validated (not partial)
    assert_eq!(auth_result["status"], "not_validated");

    client.shutdown();
}

// ============================================================================
// OAuth Auth Method Tests
// ============================================================================

#[test]
fn auth_status_not_configured_with_oauth_method_no_credentials() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__METHOD", "oauth");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_configured");
    assert_eq!(auth_result["auth_method"], "oauth");

    client.shutdown();
}

#[test]
fn auth_status_oauth_partial_with_only_client_id() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__METHOD", "oauth");
    env.insert("ONSHAPE_MCP_AUTH__CLIENT_ID", "my-client-id");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["status"], "not_configured");
    assert_eq!(auth_result["auth_method"], "oauth");
    assert!(
        auth_result["message"]
            .as_str()
            .is_some_and(|m| m.contains("client_secret"))
    );

    client.shutdown();
}

#[test]
fn auth_status_oauth_configured_with_client_credentials() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__METHOD", "oauth");
    env.insert("ONSHAPE_MCP_AUTH__CLIENT_ID", "my-client-id");
    env.insert("ONSHAPE_MCP_AUTH__CLIENT_SECRET", "my-client-secret");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "oauth");
    // With test isolation, no token file exists → OAuthPending → "not_configured"
    assert_eq!(
        auth_result["status"], "not_configured",
        "expected 'not_configured' (OAuthPending) with no token file"
    );

    client.shutdown();
}

#[test]
fn auth_method_oauth_via_env_var() {
    let mut env = HashMap::new();
    env.insert("ONSHAPE_MCP_AUTH__METHOD", "oauth");

    let mut client = McpTestClient::spawn_with_env(&env);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "oauth");

    client.shutdown();
}

#[test]
fn auth_method_oauth_via_cli_flag() {
    let mut client = McpTestClient::spawn_with_args(&["--auth-method", "oauth"]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "oauth");

    client.shutdown();
}

#[test]
fn auth_method_oauth_via_config_file() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    std::fs::write(
        &config_path,
        "[auth]\nmethod = \"oauth\"\nclient_id = \"file-cid\"\nclient_secret = \"file-cs\"\n",
    )
    .expect("should write config file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("should set permissions");
    }

    let config_arg = format!("--config={}", config_path.display());
    let mut client = McpTestClient::spawn_with_args(&[&config_arg]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "oauth");
    // With test isolation, no token file exists → OAuthPending → "not_configured"
    assert_eq!(
        auth_result["status"], "not_configured",
        "expected 'not_configured' (OAuthPending) with no token file"
    );

    client.shutdown();
}

#[test]
fn oauth_client_credentials_via_cli_flags() {
    let mut client = McpTestClient::spawn_with_args(&[
        "--auth-method",
        "oauth",
        "--client-id",
        "cli-client-id",
        "--client-secret",
        "cli-client-secret",
    ]);
    client.initialize();

    let auth_result = call_auth_status(&mut client);
    assert_eq!(auth_result["auth_method"], "oauth");
    // With test isolation, no token file exists → OAuthPending → "not_configured"
    assert_eq!(
        auth_result["status"], "not_configured",
        "expected 'not_configured' (OAuthPending) with no token file"
    );

    client.shutdown();
}

// ============================================================================
// API Tools Integration Tests
// ============================================================================

#[test]
fn tools_list_includes_api_tools() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request("tools/list", &serde_json::json!({}));
    assert!(response["error"].is_null(), "unexpected error: {response}");

    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    let mut tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    tool_names.sort_unstable();
    assert_eq!(
        tool_names,
        vec![
            "onshape_api_call",
            "onshape_api_explain",
            "onshape_api_search",
            "onshape_list_resources",
            "onshape_mcp_auth_login",
            "onshape_mcp_auth_status",
            "onshape_read_resource",
        ],
        "tool list should contain exactly the expected tools"
    );

    client.shutdown();
}

#[test]
fn api_search_returns_results_for_document_query() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_api_search",
            "arguments": {
                "query": "document",
                "tag": "Document"
            }
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");

    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");
    let results: Vec<serde_json::Value> =
        serde_json::from_str(text).expect("should be a JSON array");

    assert!(
        !results.is_empty(),
        "should find document-related endpoints"
    );
    assert!(
        results.iter().any(|r| r["operation_id"]
            .as_str()
            .is_some_and(|id| id.contains("Document") || id.contains("document"))),
        "results should contain document endpoints"
    );

    client.shutdown();
}

#[test]
fn api_explain_returns_endpoint_detail() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_api_explain",
            "arguments": {
                "endpoint": "getDocuments"
            }
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");

    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");
    let detail: serde_json::Value = serde_json::from_str(text).expect("should be valid JSON");

    assert_eq!(detail["operation_id"], "getDocuments");
    assert_eq!(detail["method"], "GET");
    assert!(
        detail["parameters"]
            .as_array()
            .is_some_and(|p| !p.is_empty()),
        "should have parameters"
    );

    client.shutdown();
}

#[test]
fn api_explain_nonexistent_returns_error() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_api_explain",
            "arguments": {
                "endpoint": "totallyFakeEndpoint"
            }
        }),
    );

    assert!(
        response["error"].is_null(),
        "should not return a protocol-level error: {response}"
    );
    assert_eq!(
        response["result"]["isError"], true,
        "should return a tool-level error for nonexistent endpoint"
    );

    client.shutdown();
}

#[test]
fn api_call_without_credentials_returns_not_configured_error() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_api_call",
            "arguments": {
                "endpoint": "getDocuments"
            }
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");

    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");

    assert!(
        text.contains("credentials are not configured"),
        "should indicate credentials are not configured, got: {text}"
    );

    // is_error should be true since credentials aren't available
    assert_eq!(
        response["result"]["isError"], true,
        "should indicate an error since credentials aren't configured"
    );

    client.shutdown();
}

// ============================================================================
// Resource Tests
// ============================================================================

#[test]
fn initialization_advertises_resources_capability() {
    let mut client = McpTestClient::spawn();
    let response = client.initialize();

    assert!(
        response["result"]["capabilities"]["resources"].is_object(),
        "resources capability should be enabled"
    );

    client.shutdown();
}

#[test]
fn resources_list_returns_insight_resources() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request("resources/list", &serde_json::json!({}));

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let resources = response["result"]["resources"]
        .as_array()
        .expect("resources should be an array");

    assert!(
        resources.len() >= 2,
        "should have at least 2 resources, got {}",
        resources.len()
    );

    // Check that shaded-views is present
    let shaded_views = resources
        .iter()
        .find(|r| r["uri"] == "insights:shaded-views")
        .expect("should have insights:shaded-views resource");
    assert_eq!(shaded_views["name"], "shaded-views");
    assert_eq!(shaded_views["mimeType"], "text/markdown");
    assert!(
        shaded_views["annotations"]["audience"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "assistant")),
        "audience should include assistant"
    );

    // Check that sketch is present
    let sketch = resources
        .iter()
        .find(|r| r["uri"] == "insights:sketch")
        .expect("should have insights:sketch resource");
    assert_eq!(sketch["name"], "sketch");

    client.shutdown();
}

#[test]
fn resources_read_returns_content_for_valid_uri() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "resources/read",
        &serde_json::json!({
            "uri": "insights:shaded-views"
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let contents = response["result"]["contents"]
        .as_array()
        .expect("contents should be an array");
    assert_eq!(contents.len(), 1, "should have exactly one content entry");

    let content = &contents[0];
    assert_eq!(content["uri"], "insights:shaded-views");
    assert_eq!(content["mimeType"], "text/markdown");

    let text = content["text"].as_str().expect("text should be a string");
    assert!(
        text.contains("Part Studio Shaded Views"),
        "content should contain the document title"
    );
    assert!(
        text.contains("pixelSize"),
        "content should contain pixelSize information"
    );

    client.shutdown();
}

#[test]
fn resources_read_returns_error_for_unknown_uri() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "resources/read",
        &serde_json::json!({
            "uri": "nonexistent:nothing"
        }),
    );

    assert!(
        response["error"].is_object(),
        "should return an error for unknown URI"
    );
    let error_message = response["error"]["message"]
        .as_str()
        .expect("error message should be a string");
    assert!(
        error_message.contains("not found"),
        "error should mention not found, got: {error_message}"
    );

    client.shutdown();
}

// ============================================================================
// Resource Tool Tests
// ============================================================================

#[test]
fn tools_list_includes_resource_tools() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request("tools/list", &serde_json::json!({}));

    assert!(response["error"].is_null(), "unexpected error: {response}");

    let tools = response["result"]["tools"]
        .as_array()
        .expect("tools should be an array");

    assert!(
        tools.iter().any(|t| t["name"] == "onshape_list_resources"),
        "should include onshape_list_resources tool"
    );
    assert!(
        tools.iter().any(|t| t["name"] == "onshape_read_resource"),
        "should include onshape_read_resource tool"
    );

    client.shutdown();
}

#[test]
fn tools_call_list_resources_returns_entries() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    // Intentionally omit the "arguments" key to verify the server
    // tolerates absent arguments for no-parameter tools.
    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_list_resources"
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");
    assert_eq!(response["result"]["isError"], false);

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");
    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");

    assert!(
        text.contains("insights:shaded-views"),
        "should list shaded-views URI, got: {text}"
    );
    assert!(
        text.contains("insights:sketch"),
        "should list sketch URI, got: {text}"
    );

    client.shutdown();
}

#[test]
fn tools_call_read_resource_returns_content() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_read_resource",
            "arguments": {
                "uri": "insights:shaded-views"
            }
        }),
    );

    assert!(response["error"].is_null(), "unexpected error: {response}");
    assert_eq!(response["result"]["isError"], false);

    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(!content.is_empty(), "content should not be empty");
    let text = content[0]["text"]
        .as_str()
        .expect("text should be a string");

    assert!(
        text.contains("Part Studio Shaded Views"),
        "should contain the document title"
    );
    assert!(
        text.contains("pixelSize"),
        "should contain pixelSize guidance"
    );

    client.shutdown();
}

#[test]
fn tools_call_read_resource_unknown_uri_returns_error() {
    let mut client = McpTestClient::spawn();
    client.initialize();

    let response = client.send_request(
        "tools/call",
        &serde_json::json!({
            "name": "onshape_read_resource",
            "arguments": {
                "uri": "nonexistent:nothing"
            }
        }),
    );

    assert!(
        response["error"].is_null(),
        "should not return a protocol-level error: {response}"
    );
    assert_eq!(
        response["result"]["isError"], true,
        "should return a tool-level error for unknown URI"
    );
    let content = response["result"]["content"]
        .as_array()
        .expect("content should be an array");
    assert!(
        !content.is_empty(),
        "content should contain at least one error entry"
    );
    let error_text = content[0]["text"]
        .as_str()
        .expect("error text should be a string");
    assert!(
        error_text.contains("not found"),
        "error should mention not found, got: {error_text}"
    );

    client.shutdown();
}
