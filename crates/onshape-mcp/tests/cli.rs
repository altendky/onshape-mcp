//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::process::Command;

use onshape_mcp::CATCH_PHRASE;

#[test]
fn binary_runs_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_onshape-mcp"))
        .output()
        .expect("failed to execute binary");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), CATCH_PHRASE);
}
