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

An absolute `XDG_CONFIG_HOME` overrides these defaults on every platform.

> **Security note:** Authentication and HTTP OAuth settings can contain
> sensitive credentials, including `secret_key`, `client_secret`, and
> `onshape_client_secret`. To protect them:
>
> - Prefer environment variables populated by your own OS or deployment secret
>   manager. The application does not yet integrate directly with OS keychains.
> - Add the config file path to `.gitignore`
> - On Unix, restrict permissions: `chmod 600 ~/.config/onshape-mcp/config.toml`
> - Avoid secret CLI flags when practical; shell history and process listings
>   may expose their values
>
> See [#19](https://github.com/altendky/onshape-mcp/issues/19) for improved credential handling
> patterns.

Example config file:

```toml
[auth]
client_id = "your-onshape-oauth-client-id"
client_secret = "your-onshape-oauth-client-secret"
method = "oauth"
check_interval = "5m"

[api]
timeout = "30s"

# Planned — not yet implemented. See the implementation roadmap.
# [mode]
# max = "read"
# initial = "read"
# allow_escalation = false
```

`auth.proxy_url` has no default. Set it only when explicitly using a proxy you
self-host, for example `proxy_url = "https://oauth-proxy.example.com"`.

Experimental self-hosted Streamable HTTP configuration:

```toml
[http]
host = "127.0.0.1"
port = 8080
public_url = "https://mcp.example.com"
onshape_client_id = "operator-owned-client-id"
onshape_client_secret = "operator-owned-client-secret"

[[http.allowed_users]]
id = "onshape-user-id"
name = "Optional display name"
```

`public_url` is the origin without `/mcp`, a query, or fragment. Clients use
`https://mcp.example.com/mcp`. Production deployments commonly bind
`0.0.0.0` behind TLS termination. An empty `allowed_users` list denies all
users. These settings are implemented but experimental; no public deployment
is provided, and ChatGPT failure is tracked in
[#546](https://github.com/altendky/onshape-mcp/issues/546).

## Environment Variables

All environment variables use the `ONSHAPE_MCP_` prefix.

## All Settings Reference

| Setting | Type | Default | Env Var | Config Key | CLI Flag | Description |
| --------- | ------ | --------- | --------- | ------------ | ---------- | ------------- |
| Access Key | `string` | — | `ONSHAPE_MCP_AUTH__ACCESS_KEY` | `auth.access_key` | `--access-key` | Onshape API access key |
| Secret Key | `string` | — | `ONSHAPE_MCP_AUTH__SECRET_KEY` | `auth.secret_key` | `--secret-key` | Onshape API secret key |
| Client ID | `string` | — | `ONSHAPE_MCP_AUTH__CLIENT_ID` | `auth.client_id` | `--client-id` | OAuth 2.0 client ID |
| Client Secret | `string` | — | `ONSHAPE_MCP_AUTH__CLIENT_SECRET` | `auth.client_secret` | `--client-secret` | OAuth 2.0 client secret |
| Self-hosted Proxy URL | `string` | — | `ONSHAPE_MCP_AUTH__PROXY_URL` | `auth.proxy_url` | `auth login --proxy-url` | Optional explicit self-hosted OAuth proxy; no runtime default |
| Auth Method | `string` | `auto` | `ONSHAPE_MCP_AUTH__METHOD` | `auth.method` | `--auth-method` | Authentication method (`auto`, `basic`, `oauth`; HMAC planned) |
| Auth Check Interval | `duration` | `5m` | `ONSHAPE_MCP_AUTH__CHECK_INTERVAL` | `auth.check_interval` | — | Periodic credential validation interval (minimum: 15s) |
| API Timeout | `duration` | `30s` | `ONSHAPE_MCP_API__TIMEOUT` | `api.timeout` | — | Request timeout for Onshape API calls |
| HTTP Host | `string` | `127.0.0.1` | `ONSHAPE_MCP_HTTP__HOST` | `http.host` | `http --host` | Experimental Streamable HTTP listen address |
| HTTP Port | `u16` | `8080` | `ONSHAPE_MCP_HTTP__PORT` | `http.port` | `http --port` | Experimental Streamable HTTP listen port |
| HTTP Public URL | `URL` | — | `ONSHAPE_MCP_HTTP__PUBLIC_URL` | `http.public_url` | `http --public-url` | Required origin used for OAuth metadata and callback URLs |
| HTTP Onshape Client ID | `string` | — | `ONSHAPE_MCP_HTTP__ONSHAPE_CLIENT_ID` | `http.onshape_client_id` | `http --onshape-client-id` | Required operator-owned OAuth client ID |
| HTTP Onshape Client Secret | `string` | — | `ONSHAPE_MCP_HTTP__ONSHAPE_CLIENT_SECRET` | `http.onshape_client_secret` | `http --onshape-client-secret` | Required operator-owned OAuth client secret |
| HTTP Allowed Users | user list | empty (deny all) | `ONSHAPE_MCP_HTTP__ALLOWED_USERS` | `http.allowed_users` | `http --allowed-users` | Onshape user IDs; env/CLI accepts `id:name,id2:name2` |
| Max Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_MAX_MODE` | `mode.max` | — | *Planned, not yet implemented.* Upper limit for permission mode |
| Initial Mode | `read`/`modify`/`destroy` | `read` | `ONSHAPE_MCP_INITIAL_MODE` | `mode.initial` | — | *Planned, not yet implemented.* Starting permission mode (must be ≤ max_mode) |
| Allow Mode Escalation | `bool` | `false` | `ONSHAPE_MCP_ALLOW_ESCALATION` | `mode.allow_escalation` | — | *Planned, not yet implemented.* Can AI change mode at runtime? |

### Token File Location

OAuth tokens are stored separately from configuration:

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.local/share/onshape-mcp/tokens.json` |
| macOS | `~/Library/Application Support/onshape-mcp/tokens.json` |
| Windows | `%LOCALAPPDATA%\onshape-mcp\tokens.json` |

An absolute `XDG_DATA_HOME` overrides these defaults on every platform. Windows
token storage otherwise always uses LocalAppData; old token files under
RoamingAppData (`%APPDATA%`) are ignored.
