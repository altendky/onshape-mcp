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
check_interval = "5m"

[mode]
max = "read"
initial = "read"
allow_escalation = false
```

## Environment Variables

All environment variables use the `ONSHAPE_MCP_` prefix.

## All Settings Reference

| Setting | Type | Default | Env Var | Config Key | Description |
| --------- | ------ | --------- | --------- | ------------ | ------------- |
| Access Key | `string` | — | `ONSHAPE_MCP_ACCESS_KEY` | `auth.access_key` | Onshape API access key |
| Secret Key | `string` | — | `ONSHAPE_MCP_SECRET_KEY` | `auth.secret_key` | Onshape API secret key |
| Max Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_MAX_MODE` | `mode.max` | Upper limit for permission mode |
| Initial Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_INITIAL_MODE` | `mode.initial` | Starting permission mode (must be ≤ max_mode) |
| Allow Mode Escalation | `bool` | `false` | `ONSHAPE_MCP_ALLOW_ESCALATION` | `mode.allow_escalation` | Can AI change mode at runtime? |
| Auth Check Interval | `duration` | `5m` | `ONSHAPE_MCP_AUTH_CHECK_INTERVAL` | `auth.check_interval` | Periodic credential validation interval (minimum: 15s) |
