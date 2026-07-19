//! Onshape MCP Server
//!
//! A Model Context Protocol server for Onshape CAD integration.
//!
//! When run without a subcommand, starts the MCP server on stdio transport.
//! Use `auth login` to complete the OAuth authorization flow.

use clap::{Parser, Subcommand};

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// Run the experimental MCP server over self-hosted Streamable HTTP transport.
    ///
    /// Serves the MCP endpoint at `/mcp` with per-user OAuth authentication
    /// via Onshape. Requires `public_url`, `onshape_client_id`, and
    /// `onshape_client_secret` to be configured.
    Http {
        /// Listen address (overrides config file).
        #[arg(long)]
        host: Option<String>,

        /// Listen port (overrides config file).
        #[arg(long)]
        port: Option<u16>,

        /// Public URL of this server (overrides config file).
        #[arg(long)]
        public_url: Option<String>,

        /// Onshape OAuth app client ID (overrides config file).
        #[arg(long)]
        onshape_client_id: Option<String>,

        /// Onshape OAuth app client secret (overrides config file).
        #[arg(long)]
        onshape_client_secret: Option<String>,

        /// Comma-separated list of allowed users (id:name pairs).
        ///
        /// Overrides config file. Example: `--allowed-users "abc123:Alice,def456:Bob"`
        #[arg(long)]
        allowed_users: Option<String>,
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
        /// Use an explicitly self-hosted OAuth proxy instead of direct OAuth.
        #[arg(long, value_parser = parse_nonblank_proxy_url)]
        proxy_url: Option<String>,
    },
}

fn parse_nonblank_proxy_url(value: &str) -> Result<String, String> {
    onshape_mcp_core::validate_proxy_url(value)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Auth { ref action }) => handle_auth_command(action, &cli).await,
        Some(Command::Http {
            ref host,
            ref port,
            ref public_url,
            ref onshape_client_id,
            ref onshape_client_secret,
            ref allowed_users,
        }) => {
            run_http_server(
                &cli,
                host.clone(),
                *port,
                public_url.clone(),
                onshape_client_id.clone(),
                onshape_client_secret.clone(),
                allowed_users.clone(),
            )
            .await
        }
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

/// Run the MCP server over Streamable HTTP transport.
#[allow(clippy::too_many_arguments)]
async fn run_http_server(
    cli: &Cli,
    host: Option<String>,
    port: Option<u16>,
    public_url: Option<String>,
    onshape_client_id: Option<String>,
    onshape_client_secret: Option<String>,
    allowed_users: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli_overrides = build_cli_overrides(cli);

    let mut config =
        onshape_mcp_io::config::load_config_with_overrides(cli.config.as_deref(), cli_overrides)?;

    // Apply HTTP-specific CLI overrides.
    if let Some(h) = host {
        config.http.host = h;
    }
    if let Some(p) = port {
        config.http.port = p;
    }
    if let Some(url) = public_url {
        config.http.public_url = Some(url);
    }
    if let Some(id) = onshape_client_id {
        config.http.onshape_client_id = Some(id);
    }
    if let Some(secret) = onshape_client_secret {
        config.http.onshape_client_secret = Some(secrecy::SecretString::from(secret));
    }
    if let Some(users_csv) = allowed_users {
        config.http.allowed_users = onshape_mcp_core::config::parse_allowed_users_csv(&users_csv);
    }

    onshape_mcp_io::run_http(NAME, VERSION, config).await
}

/// Handle `auth` subcommands.
async fn handle_auth_command(
    action: &AuthCommand,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        AuthCommand::Login { proxy_url } => handle_auth_login(proxy_url.clone(), cli).await,
    }
}

/// Handle `auth login` — complete the OAuth authorization flow.
async fn handle_auth_login(
    proxy_url: Option<String>,
    cli: &Cli,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use onshape_mcp_core::tools::LoginMode;

    // Supplying a proxy URL explicitly selects self-hosted proxy mode.
    let mode = if let Some(proxy_url) = proxy_url {
        LoginMode::Proxy { proxy_url }
    } else {
        let (client_id, client_secret) = resolve_direct_credentials(cli)?;
        LoginMode::Direct {
            client_id,
            client_secret,
        }
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
        return validate_direct_credentials(id.clone(), secret.clone());
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

    validate_direct_credentials(client_id, client_secret)
}

fn validate_direct_credentials(
    client_id: String,
    client_secret: String,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    if client_id.trim().is_empty() {
        return Err("client_id must not be blank".into());
    }
    if client_secret.trim().is_empty() {
        return Err("client_secret must not be blank".into());
    }

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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{AuthCommand, Cli, Command, resolve_direct_credentials};

    #[test]
    fn bare_auth_login_has_no_proxy_url() {
        let cli = Cli::try_parse_from(["onshape-mcp", "auth", "login"]).expect("should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Auth {
                action: AuthCommand::Login { proxy_url: None }
            })
        ));
    }

    #[test]
    fn auth_login_proxy_url_is_explicit() {
        let cli = Cli::try_parse_from([
            "onshape-mcp",
            "auth",
            "login",
            "--proxy-url",
            "https://oauth-proxy.example.com",
        ])
        .expect("should parse");

        assert!(matches!(
            cli.command,
            Some(Command::Auth {
                action: AuthCommand::Login {
                    proxy_url: Some(ref url)
                }
            }) if url == "https://oauth-proxy.example.com"
        ));
    }

    #[test]
    fn auth_login_direct_flag_is_removed() {
        let Err(error) = Cli::try_parse_from(["onshape-mcp", "auth", "login", "--direct"]) else {
            panic!("--direct should not be accepted");
        };

        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn auth_login_rejects_blank_proxy_url() {
        let Err(error) = Cli::try_parse_from(["onshape-mcp", "auth", "login", "--proxy-url", "  "])
        else {
            panic!("blank --proxy-url should not be accepted");
        };

        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn auth_login_rejects_insecure_non_loopback_proxy_url() {
        let Err(error) = Cli::try_parse_from([
            "onshape-mcp",
            "auth",
            "login",
            "--proxy-url",
            "http://proxy.example.com",
        ]) else {
            panic!("insecure remote proxy should fail");
        };

        assert_eq!(error.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(error.to_string().contains("https://"));
    }

    #[test]
    fn auth_login_allows_http_loopback_proxy_url() {
        for proxy_url in ["http://localhost:8787", "http://127.8.9.10", "http://[::1]"] {
            Cli::try_parse_from(["onshape-mcp", "auth", "login", "--proxy-url", proxy_url])
                .expect("loopback HTTP proxy should parse");
        }
    }

    #[test]
    fn direct_credentials_reject_blank_client_id() {
        let cli = Cli::try_parse_from([
            "onshape-mcp",
            "--client-id",
            "  ",
            "--client-secret",
            "secret",
            "auth",
            "login",
        ])
        .expect("should parse");

        let error = resolve_direct_credentials(&cli).expect_err("blank client ID should fail");
        assert!(error.to_string().contains("client_id"));
    }

    #[test]
    fn direct_credentials_reject_blank_client_secret() {
        let cli = Cli::try_parse_from([
            "onshape-mcp",
            "--client-id",
            "client",
            "--client-secret",
            "\t ",
            "auth",
            "login",
        ])
        .expect("should parse");

        let error = resolve_direct_credentials(&cli).expect_err("blank client secret should fail");
        assert!(error.to_string().contains("client_secret"));
    }

    #[test]
    fn direct_credentials_preserve_nonblank_client_secret() {
        let cli = Cli::try_parse_from([
            "onshape-mcp",
            "--client-id",
            "client",
            "--client-secret",
            " secret with spaces ",
            "auth",
            "login",
        ])
        .expect("should parse");

        let (_, secret) = resolve_direct_credentials(&cli).expect("credentials should resolve");
        assert_eq!(secret, " secret with spaces ");
    }

    #[test]
    fn http_help_marks_transport_experimental_and_self_hosted() {
        let mut command = Cli::command();
        let http = command
            .find_subcommand_mut("http")
            .expect("http subcommand should exist");
        let help = http
            .get_about()
            .expect("http help should exist")
            .to_string();

        assert!(help.contains("experimental"));
        assert!(help.contains("self-hosted"));
    }
}
