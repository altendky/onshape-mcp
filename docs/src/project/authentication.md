# Authentication

## Supported Methods

| Method | Status | Notes |
| -------- | -------- | ------- |
| API Keys | Initial implementation | Personal use, single user |
| OAuth 2.0 | Future | Multi-user apps, team access |

## Credential Sources

Credentials can be provided via config file or environment variables. See [Configuration](configuration.md) for file locations and precedence rules.

**Config file example:**

```toml
[auth]
access_key = "..."
secret_key = "..."
```

**Environment variables:**

| Variable | Description |
| ---------- | ------------- |
| `ONSHAPE_MCP_ACCESS_KEY` | Onshape API access key |
| `ONSHAPE_MCP_SECRET_KEY` | Onshape API secret key |

**Future credential sources** (to be implemented):

- System keychain integration

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
