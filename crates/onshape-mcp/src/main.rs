//! Onshape MCP Server
//!
//! A Model Context Protocol server for Onshape CAD integration.

use clap::Parser;

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Onshape MCP Server — A Model Context Protocol server for Onshape CAD integration.
#[derive(Parser)]
#[command(name = NAME, version = VERSION)]
struct Args {
    /// Onshape API access key (overrides config file and environment variable).
    #[arg(long)]
    access_key: Option<String>,

    /// Onshape API secret key (overrides config file and environment variable).
    #[arg(long)]
    secret_key: Option<String>,

    /// Authentication method for Onshape API requests (overrides config file and environment variable).
    #[arg(long)]
    auth_method: Option<String>,

    /// Path to config file (default: ~/.config/onshape-mcp/config.toml).
    #[arg(long)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Build CLI overrides dict for figment (highest priority).
    // Structure: { "auth": { "access_key": "...", "secret_key": "..." } }
    let mut auth_overrides = figment::value::Dict::new();
    if let Some(ref access_key) = args.access_key {
        auth_overrides.insert(
            "access_key".into(),
            figment::value::Value::from(access_key.clone()),
        );
    }
    if let Some(ref secret_key) = args.secret_key {
        auth_overrides.insert(
            "secret_key".into(),
            figment::value::Value::from(secret_key.clone()),
        );
    }
    if let Some(ref auth_method) = args.auth_method {
        auth_overrides.insert(
            "method".into(),
            figment::value::Value::from(auth_method.clone()),
        );
    }

    let mut cli_overrides = figment::value::Dict::new();
    if !auth_overrides.is_empty() {
        cli_overrides.insert("auth".into(), figment::value::Value::from(auth_overrides));
    }

    let config =
        onshape_mcp_io::config::load_config_with_overrides(args.config.as_deref(), cli_overrides)?;

    onshape_mcp_io::run(NAME, VERSION, config).await
}
