# Authentication

## Supported Methods

| Method | Status | Notes |
| -------- | -------- | ------- |
| API Keys (Basic) | Implemented | Base64-encoded credentials over HTTPS; personal use, single user |
| API Keys (HMAC-SHA256) | Future | Per-request signed headers with nonce/timestamp; replay protection, secret never sent |
| OAuth 2.0 | Implemented | Authorization code flow via OpenCode plugin; token file storage |

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
| `ONSHAPE_MCP_AUTH__METHOD` | Set to `oauth` to use OAuth authentication |
| `ONSHAPE_MCP_AUTH__CLIENT_ID` | OAuth 2.0 client ID |
| `ONSHAPE_MCP_AUTH__CLIENT_SECRET` | OAuth 2.0 client secret |

**CLI flags:**

| Flag | Description |
| ------ | ------------- |
| `--auth-method oauth` | Use OAuth authentication |
| `--client-id <ID>` | OAuth 2.0 client ID |
| `--client-secret <SECRET>` | OAuth 2.0 client secret |

### OAuth Token File

OAuth access and refresh tokens are stored in a local JSON file:

| Platform | Location |
| ---------- | ---------- |
| Unix | `~/.local/share/onshape-mcp/tokens.json` |
| macOS | `~/Library/Application Support/onshape-mcp/tokens.json` |
| Windows | `%LOCALAPPDATA%\onshape-mcp\tokens.json` |

The token file has the same permission requirements as the config file (0600 on Unix).
Token refresh is not yet implemented; when tokens expire, re-run the OAuth authorization flow.

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

## Ecosystem Context

Our security approach exceeds MCP ecosystem norms:

- **File permission enforcement** — No surveyed MCP servers enforce config file permissions; we block access if permissions are too open
- **Environment variables are standard** — Most MCP servers rely solely on environment variables for credentials
- **No keychain implementations** — System keychain integration is an ecosystem gap we plan to address

The MCP specification provides security best practices but does not mandate credential storage mechanisms.
See [SEP-1024](https://modelcontextprotocol.io/community/seps/1024-mcp-client-security-requirements-for-local-server-.md) (client security for local servers) and [SEP-1046](https://modelcontextprotocol.io/community/seps/1046-support-oauth-client-credentials-flow-in-authoriza.md) (OAuth client credentials flow) for related guidance.
