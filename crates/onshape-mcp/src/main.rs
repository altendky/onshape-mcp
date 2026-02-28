//! Onshape MCP Server
//!
//! A Model Context Protocol server for Onshape CAD integration.
//!
//! When run without a subcommand, starts the MCP server on stdio transport.
//! Use `auth login` to complete the OAuth authorization flow.

use clap::{Parser, Subcommand};

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default OAuth proxy URL used when no `proxy_url` is specified.
const DEFAULT_PROXY_URL: &str = "https://onshape-oauth-proxy.fstab.workers.dev";

/// Onshape MCP Server — A Model Context Protocol server for Onshape CAD integration.
#[derive(Parser)]
#[command(name = NAME, version = VERSION)]
struct Cli {
    /// Onshape API access key (overrides config file and environment variable).
    #[arg(long)]
    access_key: Option<String>,

    /// Onshape API secret key (overrides config file and environment variable).
    #[arg(long)]
    secret_key: Option<String>,

    /// OAuth 2.0 client ID (overrides config file and environment variable).
    #[arg(long)]
    client_id: Option<String>,

    /// OAuth 2.0 client secret (overrides config file and environment variable).
    #[arg(long)]
    client_secret: Option<String>,

    /// Authentication method for Onshape API requests (overrides config file and environment variable).
    #[arg(long)]
    auth_method: Option<String>,

    /// Path to config file (default: ~/.config/onshape-mcp/config.toml).
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Subcommand to run. When omitted, starts the MCP server.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Command {
    /// Authentication management.
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },
}

/// Authentication subcommands.
#[derive(Subcommand)]
enum AuthCommand {
    /// Complete the OAuth authorization flow.
    ///
    /// Opens your browser to authorize with Onshape, then exchanges
    /// the authorization code for tokens and saves them to the token file.
    Login {
        /// Use direct mode (provide `client_id` and `client_secret` directly).
        /// By default, uses the OAuth proxy which holds the client secret.
        #[arg(long)]
        direct: bool,

        /// OAuth proxy URL override (only used in proxy mode).
        #[arg(long)]
        proxy_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Auth { ref action }) => handle_auth_command(action, &cli).await,
        None => run_server(cli).await,
    }
}

/// Run the MCP server (default behavior when no subcommand is given).
async fn run_server(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli_overrides = build_cli_overrides(&cli);

    let config =
        onshape_mcp_io::config::load_config_with_overrides(cli.config.as_deref(), cli_overrides)
            .map_err(|e| {
                if cli.auth_method.is_some()
                    && let onshape_mcp_io::config::ConfigLoadError::Figment(ref figment_err) = e
                {
                    let auth_method_path = &["auth", "method"];
                    let is_auth_method_error = figment_err.clone().into_iter().any(|err| {
                        err.path.len() >= auth_method_path.len()
                            && err.path[..auth_method_path.len()]
                                .iter()
                                .zip(auth_method_path)
                                .all(|(a, b)| a == b)
                    });
                    if is_auth_method_error {
                        clap::Error::raw(
                            clap::error::ErrorKind::InvalidValue,
                            format!("invalid value for '--auth-method': {e}\n"),
                        )
                        .exit();
                    }
                }
                e
            })?;

    onshape_mcp_io::run(NAME, VERSION, config).await
}

/// Handle `auth` subcommands.
async fn handle_auth_command(
    action: &AuthCommand,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        AuthCommand::Login { direct, proxy_url } => {
            handle_auth_login(*direct, proxy_url.clone(), cli).await
        }
    }
}

/// Handle `auth login` — complete the OAuth authorization flow.
async fn handle_auth_login(
    direct: bool,
    proxy_url: Option<String>,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use onshape_mcp_core::tools::LoginMode;

    // Determine the login mode from flags and config.
    let mode = if direct {
        // Direct mode: need client_id and client_secret.
        // Try CLI flags first, then load from config.
        let (client_id, client_secret) = resolve_direct_credentials(cli)?;
        LoginMode::Direct {
            client_id,
            client_secret,
        }
    } else {
        // Proxy mode.
        let url = proxy_url.unwrap_or_else(|| DEFAULT_PROXY_URL.to_string());
        LoginMode::Proxy { proxy_url: url }
    };

    eprintln!("Starting OAuth authorization flow...");

    // Start the login flow.
    let handle = onshape_mcp_io::login::start_login_flow(&mode).await?;

    eprintln!("Opening browser for authorization...");
    eprintln!();
    eprintln!("If the browser does not open, visit this URL:");
    eprintln!("  {}", handle.authorize_url);
    eprintln!();

    // Try to open the browser (best effort — don't fail if it doesn't work).
    let _ = open::that(&handle.authorize_url);

    eprintln!("Waiting for authorization (timeout: 2 minutes)...");

    // Wait for the flow to complete.
    match handle.result_rx.await {
        Ok(Ok(())) => {
            eprintln!();
            eprintln!("Authorization successful! Tokens saved.");
            eprintln!("The MCP server will automatically detect the new tokens.");
            Ok(())
        }
        Ok(Err(e)) => {
            eprintln!();
            eprintln!("Authorization failed: {e}");
            Err(e.into())
        }
        Err(_) => {
            eprintln!();
            eprintln!("Authorization flow was interrupted.");
            Err("login flow interrupted".into())
        }
    }
}

/// Resolve `client_id` and `client_secret` for direct mode.
///
/// Checks CLI flags first, then falls back to config file / env vars.
fn resolve_direct_credentials(
    cli: &Cli,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    use secrecy::ExposeSecret;

    // Try CLI flags first.
    if let (Some(id), Some(secret)) = (&cli.client_id, &cli.client_secret) {
        return Ok((id.clone(), secret.clone()));
    }

    // Fall back to config file / env vars.
    let cli_overrides = build_cli_overrides(cli);
    let config =
        onshape_mcp_io::config::load_config_with_overrides(cli.config.as_deref(), cli_overrides)?;

    let client_id = config.auth.client_id.ok_or(
        "client_id is required for direct mode. \
         Provide via --client-id flag, config file, or ONSHAPE_MCP_AUTH__CLIENT_ID env var.",
    )?;

    let client_secret_value = config.auth.client_secret.ok_or(
        "client_secret is required for direct mode. \
         Provide via --client-secret flag, config file, or ONSHAPE_MCP_AUTH__CLIENT_SECRET env var.",
    )?;
    let client_secret = client_secret_value.expose_secret().to_string();

    Ok((client_id, client_secret))
}

/// Build the figment CLI overrides dict from the CLI args.
fn build_cli_overrides(cli: &Cli) -> figment::value::Dict {
    let mut auth_overrides = figment::value::Dict::new();
    if let Some(ref access_key) = cli.access_key {
        auth_overrides.insert(
            "access_key".into(),
            figment::value::Value::from(access_key.clone()),
        );
    }
    if let Some(ref secret_key) = cli.secret_key {
        auth_overrides.insert(
            "secret_key".into(),
            figment::value::Value::from(secret_key.clone()),
        );
    }
    if let Some(ref client_id) = cli.client_id {
        auth_overrides.insert(
            "client_id".into(),
            figment::value::Value::from(client_id.clone()),
        );
    }
    if let Some(ref client_secret) = cli.client_secret {
        auth_overrides.insert(
            "client_secret".into(),
            figment::value::Value::from(client_secret.clone()),
        );
    }
    if let Some(ref auth_method) = cli.auth_method {
        auth_overrides.insert(
            "method".into(),
            figment::value::Value::from(auth_method.clone()),
        );
    }

    let mut cli_overrides = figment::value::Dict::new();
    if !auth_overrides.is_empty() {
        cli_overrides.insert("auth".into(), figment::value::Value::from(auth_overrides));
    }
    cli_overrides
}
