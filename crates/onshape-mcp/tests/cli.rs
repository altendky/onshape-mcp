//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use onshape_mcp::CATCH_PHRASE;

/// Find the binary path, handling both normal cargo test and nextest archive contexts.
fn find_binary() -> PathBuf {
    // First try the compile-time path (works with `cargo test`)
    let compile_time_path = PathBuf::from(env!("CARGO_BIN_EXE_onshape-mcp"));
    if compile_time_path.exists() {
        return compile_time_path;
    }

    // For nextest archives, the binary is extracted relative to the workspace root.
    // The test binary knows its own location, so we can find the main binary relative to it.
    let current_exe = std::env::current_exe().expect("failed to get current exe path");

    // The test binary is in target/debug/deps/, the main binary is in target/debug/
    // Go up from deps to debug, then look for the binary
    if let Some(deps_dir) = current_exe.parent() {
        if let Some(debug_dir) = deps_dir.parent() {
            let binary_name = if cfg!(windows) {
                "onshape-mcp.exe"
            } else {
                "onshape-mcp"
            };
            let binary_path = debug_dir.join(binary_name);
            if binary_path.exists() {
                return binary_path;
            }
        }
    }

    // Fall back to compile-time path (will fail with a clear error)
    compile_time_path
}

#[test]
fn binary_runs_successfully() {
    let binary_path = find_binary();
    let output = Command::new(&binary_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute binary at {binary_path:?}: {e}"));

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), CATCH_PHRASE);
}
