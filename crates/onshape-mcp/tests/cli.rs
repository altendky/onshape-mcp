//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use onshape_mcp::CATCH_PHRASE;

/// Find the binary path, handling both normal cargo test and nextest archive contexts.
fn find_binary() -> PathBuf {
    // Runtime: nextest sets this correctly even for archives
    if let Some(path) = std::env::var_os("NEXTEST_BIN_EXE_onshape-mcp")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }

    // Fallback for regular cargo test
    PathBuf::from(env!("CARGO_BIN_EXE_onshape-mcp"))
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
