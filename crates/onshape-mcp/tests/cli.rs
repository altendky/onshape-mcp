//! Integration tests for the onshape-mcp binary.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::process::Command;

#[test]
fn binary_runs_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_onshape-mcp"))
        .status()
        .expect("failed to execute binary");
    assert!(status.success());
}
