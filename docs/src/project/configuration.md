# Configuration

## Architecture & Patterns

Configuration uses `figment` for layered configuration with `clap` for CLI argument parsing. This provides:

- Excellent error provenance (know exactly where a value came from)
- First-class serde integration
- Multiple source support with clear precedence

## Configuration Precedence

From lowest to highest priority:

1. **Defaults** (hardcoded)
2. **Config file**
3. **Environment variables**
4. **CLI flags**

## Config File

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.config/onshape-mcp/config.toml` |
| Windows | `%APPDATA%\onshape-mcp\config.toml` |

> **Security note:** The `[auth]` section contains sensitive credentials
> (`access_key` and `secret_key`). To protect them:
>
> - **Preferred:** Use your OS secret store (macOS Keychain, Windows Credential
>   Manager, or Linux secret service) and pass credentials via environment
>   variables
> - Add the config file path to `.gitignore`
> - On Unix, restrict permissions: `chmod 600 ~/.config/onshape-mcp/config.toml`
>
> See [#19](https://github.com/altendky/onshape-mcp/issues/19) for improved credential handling
> patterns.

Example config file:

```toml
[auth]
access_key = "..."
secret_key = "..."
method = "basic"
check_interval = "5m"

[http]
timeout = "30s"

[mode]
max = "read"
initial = "read"
allow_escalation = false
```

## Environment Variables

All environment variables use the `ONSHAPE_MCP_` prefix.

## All Settings Reference

| Setting | Type | Default | Env Var | Config Key | CLI Flag | Description |
| --------- | ------ | --------- | --------- | ------------ | ---------- | ------------- |
| Access Key | `string` | — | `ONSHAPE_MCP_AUTH__ACCESS_KEY` | `auth.access_key` | `--access-key` | Onshape API access key |
| Secret Key | `string` | — | `ONSHAPE_MCP_AUTH__SECRET_KEY` | `auth.secret_key` | `--secret-key` | Onshape API secret key |
| Client ID | `string` | — | `ONSHAPE_MCP_AUTH__CLIENT_ID` | `auth.client_id` | `--client-id` | OAuth 2.0 client ID |
| Client Secret | `string` | — | `ONSHAPE_MCP_AUTH__CLIENT_SECRET` | `auth.client_secret` | `--client-secret` | OAuth 2.0 client secret |
| Auth Method | `string` | `basic` | `ONSHAPE_MCP_AUTH__METHOD` | `auth.method` | `--auth-method` | Authentication method (`basic`, `oauth`; HMAC planned) |
| Auth Check Interval | `duration` | `5m` | `ONSHAPE_MCP_AUTH__CHECK_INTERVAL` | `auth.check_interval` | — | Periodic credential validation interval (minimum: 15s) |
| HTTP Timeout | `duration` | `30s` | `ONSHAPE_MCP_HTTP__TIMEOUT` | `http.timeout` | — | Request timeout for Onshape API calls |
| Max Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_MAX_MODE` | `mode.max` | — | Upper limit for permission mode |
| Initial Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_INITIAL_MODE` | `mode.initial` | — | Starting permission mode (must be ≤ max_mode) |
| Allow Mode Escalation | `bool` | `false` | `ONSHAPE_MCP_ALLOW_ESCALATION` | `mode.allow_escalation` | — | Can AI change mode at runtime? |

### Token File Location

OAuth tokens are stored separately from configuration:

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.local/share/onshape-mcp/tokens.json` |
| macOS | `~/Library/Application Support/onshape-mcp/tokens.json` |
| Windows | `%LOCALAPPDATA%\onshape-mcp\tokens.json` |
