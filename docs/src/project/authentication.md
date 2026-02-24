# Authentication

## Supported Methods

| Method | Status | Notes |
| -------- | -------- | ------- |
| Auto (default) | Implemented | Automatically detects the best available auth method |
| API Keys (Basic) | Implemented | Base64-encoded credentials over HTTPS; personal use, single user |
| API Keys (HMAC-SHA256) | Future | Per-request signed headers with nonce/timestamp; replay protection, secret never sent |
| OAuth 2.0 | Implemented | Authorization code flow via OpenCode plugin; token file storage |

## Auto-Detection (Default)

When `method` is set to `auto` (the default), the server automatically selects the best authentication method based on available credentials.
Priority order:

1. **OAuth with tokens** — Client credentials + token file present.
OAuth is preferred when fully configured because it provides scoped, revocable access.
2. **Basic auth** — Both API keys (`access_key` + `secret_key`) present.
3. **OAuth pending** — Client credentials present but no token file yet.
The server watches for the token file to appear (see [Token File Watching](#token-file-watching)).
4. **Not configured** — No complete credential set found.

To override auto-detection, set `method` explicitly to `basic` or `oauth`.

## Credential Sources

Credentials can be provided via config file or environment variables. See [Configuration](configuration.md) for file locations and precedence rules.

### API Key Credentials

**Config file example:**

```toml
[auth]
access_key = "..."
secret_key = "..."
```

**Environment variables:**

| Variable | Description |
| ---------- | ------------- |
| `ONSHAPE_MCP_AUTH__ACCESS_KEY` | Onshape API access key |
| `ONSHAPE_MCP_AUTH__SECRET_KEY` | Onshape API secret key |

### OAuth 2.0 Credentials

OAuth requires a client ID and client secret from an Onshape OAuth application, plus access tokens obtained through the authorization code flow.

**Config file example:**

```toml
[auth]
method = "oauth"
client_id = "..."
client_secret = "..."
```

**Environment variables:**

| Variable | Description |
| ---------- | ------------- |
| `ONSHAPE_MCP_AUTH__METHOD` | Auth method: `auto` (default), `basic`, or `oauth` |
| `ONSHAPE_MCP_AUTH__CLIENT_ID` | OAuth 2.0 client ID |
| `ONSHAPE_MCP_AUTH__CLIENT_SECRET` | OAuth 2.0 client secret |

**CLI flags:**

| Flag | Description |
| ------ | ------------- |
| `--auth-method <METHOD>` | Auth method: `auto` (default), `basic`, or `oauth` |
| `--client-id <ID>` | OAuth 2.0 client ID |
| `--client-secret <SECRET>` | OAuth 2.0 client secret |

### OAuth Credentials File

When the user completes `o auth login` via the OpenCode plugin, the plugin writes the OAuth client credentials to a local file so the MCP server can use them for token refresh without separate configuration.

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.local/share/onshape-mcp/credentials.json` |
| macOS | `~/Library/Application Support/onshape-mcp/credentials.json` |
| Windows | `%LOCALAPPDATA%\onshape-mcp\credentials.json` |

The credentials file has the same permission requirements as the config file (0600 on Unix).
It contains `client_id` and `client_secret` as a JSON object.
Explicit configuration (config file, environment variables, CLI flags) always takes precedence over the credentials file.

### OAuth Token File

OAuth access and refresh tokens are stored in a local JSON file:

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.local/share/onshape-mcp/tokens.json` |
| macOS | `~/Library/Application Support/onshape-mcp/tokens.json` |
| Windows | `%LOCALAPPDATA%\onshape-mcp\tokens.json` |

The token file has the same permission requirements as the config file (0600 on Unix).
Token refresh is handled automatically: the server proactively refreshes tokens before they expire and reactively refreshes on 401 responses. Refreshed tokens are persisted to the token file. The server also detects externally-refreshed tokens (e.g. from another process writing the token file).

The token file includes an optional `scopes` field tracking the OAuth scopes granted by the authorization server. The `token_type` field is validated on load — only `"bearer"` (case-insensitive) is accepted.

### Token File Watching

The server monitors the token file for changes using OS-native file watching (inotify on Linux, kqueue on macOS, `ReadDirectoryChanges` on Windows) with a polling fallback if native watching is unavailable.

Token file watching is active when the server starts in `OAuthPending` or `OAuth` state:

- **OAuthPending to OAuth transition** — When the user completes the OAuth authorization flow (e.g. via the OpenCode plugin) and the token file appears, the server automatically transitions to `OAuth` state and begins serving API requests.
No server restart is needed.
- **External token refresh** — When another process (e.g. the OpenCode plugin) refreshes the token and writes it to the file, the server picks up the new token automatically.

The watcher monitors the token file's parent directory (since the file may not exist yet) and debounces events with a 500ms window to handle the multiple filesystem events that a single write can produce.

### OAuthPending State

When OAuth client credentials are configured but no token file exists yet, the server enters the `OAuthPending` state.
In this state:

- The `auth_status` tool reports `status: "not_configured"` with `auth_method: "oauth"` and a message directing the user to complete the OAuth flow.
- API calls return an error explaining that OAuth authorization is not yet complete.
- The token file watcher is active, waiting for tokens to appear.

### OpenCode Plugin

When using OpenCode, the OAuth flow is handled by the `.opencode/plugin.ts` plugin.
The plugin prompts for client ID and client secret, opens the Onshape authorization page in your browser,
starts a local callback server, exchanges the authorization code for tokens, and writes them to the token file.

**Future credential sources** (to be implemented):

- System keychain integration via the [`keyring`](https://docs.rs/keyring) crate

| Platform | Backend | Feature Flag |
| -------- | -------- | ------- |
| macOS | Keychain Services | `apple-native` |
| Windows | Credential Manager | `windows-native` |
| Linux | Secret Service (DBus) | `sync-secret-service` |
| Linux | keyutils (kernel) | `linux-native` |

Tradeoffs: most secure (OS-managed encryption), but requires unlock prompt on first access and DBus dependency on Linux.

## Config File Security

The config file contains secrets and must have restricted permissions:

- **Unix:** `0600` (owner read/write only)
- **Windows:** File accessible only to the owner account (remove inherited permissions, clear all existing access rules, and grant only the owner read/write access)

  To set Windows permissions via PowerShell:

  ```powershell
  $path = "path\to\config.toml"
  $acl = Get-Acl $path
  $acl.SetAccessRuleProtection($true, $false)
  $acl.Access | ForEach-Object { $acl.RemoveAccessRule($_) } | Out-Null
  $rule = New-Object System.Security.AccessControl.FileSystemAccessRule(
      "$env:USERDOMAIN\$env:USERNAME", "Read,Write", "Allow")
  $acl.AddAccessRule($rule)
  Set-Acl $path $acl
  ```

  To verify, run `Get-Acl "path\to\config.toml" | Format-List` and confirm only your account has access.

If permissions are too open, the server **blocks access** and informs the user of the issue.

## Credential Validation

| Event | Behavior |
| ------- | ---------- |
| Startup | Validate credentials, fail if invalid |
| Periodic | Re-validate at configured interval (see `auth.check_interval` in [Configuration](configuration.md#all-settings-reference)) |
| API call | Updates auth status, resets periodic check timer |
| Invalid credentials | Fail API calls with clear error, emit MCP notification |

**No caching:** If credentials become invalid mid-session, all subsequent API calls fail until credentials are fixed.

## MCP Notifications

The server emits MCP notifications for auth status changes:

- `notifications/onshape/auth/invalid` — Credentials became invalid
- `notifications/onshape/auth/restored` — Credentials are valid again

## Token Revocation

Onshape does **not** expose a programmatic token revocation endpoint ([RFC 7009](https://tools.ietf.org/html/rfc7009)). The Onshape OAuth documentation only defines two endpoints:

| Endpoint | URL |
| ---------- | ----- |
| Authorization | `https://oauth.onshape.com/oauth/authorize` |
| Token | `https://oauth.onshape.com/oauth/token` |

To revoke an application's access, users must do so manually through the Onshape UI:

1. Sign in to [cad.onshape.com](https://cad.onshape.com)
2. Click your name (top-right) > **My account**
3. Click **Applications** in the left sidebar
4. Click **Revoke** next to the application

When a user revokes access, the refresh token is invalidated. The next token refresh attempt will fail, and API calls will return 401 until the user re-authorizes through the OAuth flow.

Because there is no revocation endpoint, the `oauth2` crate's `OnshapeOAuthClient` type intentionally leaves the revocation endpoint unset (`EndpointNotSet`). If Onshape adds revocation support in the future, the crate's `revoke_token()` method could be used to revoke tokens on server shutdown or credential rotation.

## Ecosystem Context

Our security approach exceeds MCP ecosystem norms:

- **File permission enforcement** — No surveyed MCP servers enforce config file permissions; we block access if permissions are too open
- **Environment variables are standard** — Most MCP servers rely solely on environment variables for credentials
- **No keychain implementations** — System keychain integration is an ecosystem gap we plan to address

The MCP specification provides security best practices but does not mandate credential storage mechanisms.
See [SEP-1024](https://modelcontextprotocol.io/community/seps/1024-mcp-client-security-requirements-for-local-server-.md) (client security for local servers) and [SEP-1046](https://modelcontextprotocol.io/community/seps/1046-support-oauth-client-credentials-flow-in-authoriza.md) (OAuth client credentials flow) for related guidance.
