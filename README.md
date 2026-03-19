# Onshape MCP

[![CI](https://github.com/altendky/onshape-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/altendky/onshape-mcp/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/onshape-mcp.svg)](https://crates.io/crates/onshape-mcp)
[![npm](https://img.shields.io/npm/v/onshape-mcp.svg)](https://www.npmjs.com/package/onshape-mcp)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

A Rust-based [MCP](https://modelcontextprotocol.io/) server that gives AI assistants access to the full [Onshape](https://www.onshape.com/) REST API. Instead of hard-coding individual API endpoints as tools (which would consume LLM context), it embeds the complete Onshape OpenAPI specification and lets the AI discover and invoke endpoints dynamically. The server provides 11 tools covering API discovery, authentication, CAD inspection, and built-in knowledge resources.

## Status

Early development (pre-release). Core functionality works: API search/explain/call, basic and OAuth 2.0 authentication, and cross-platform distribution. See the [implementation roadmap](docs/src/project/implementation.md) for what's done and what's next.

## Getting Started

You don't install or run this server directly — your MCP client (Claude Desktop, OpenCode, etc.) launches it automatically. You just need to:

1. [Configure your MCP client](#mcp-client-configuration) to know about this server
2. [Set up authentication](#authentication) so the server can talk to Onshape

### Supported Platforms

| Platform | Architecture |
| -------- | ------------ |
| Linux | x86_64, aarch64 |
| macOS | x86_64, aarch64 (Apple Silicon) |
| Windows | x86_64 |

## MCP Client Configuration

### Claude Desktop

Add to your Claude Desktop config (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "onshape": {
      "command": "npx",
      "args": ["--yes", "onshape-mcp"],
      "env": {
        "ONSHAPE_MCP_AUTH__ACCESS_KEY": "your-access-key",
        "ONSHAPE_MCP_AUTH__SECRET_KEY": "your-secret-key"
      }
    }
  }
}
```

### OpenCode

Add to your `opencode.json`:

```json
{
  "mcp": {
    "onshape": {
      "type": "local",
      "command": ["npx", "--yes", "onshape-mcp"],
      "environment": {
        "ONSHAPE_MCP_AUTH__ACCESS_KEY": "your-access-key",
        "ONSHAPE_MCP_AUTH__SECRET_KEY": "your-secret-key"
      }
    }
  }
}
```

For OAuth 2.0 with OpenCode, use the auth plugin instead (see [OAuth 2.0](#oauth-20) below):

```json
{
  "mcp": {
    "onshape": {
      "type": "local",
      "command": ["npx", "--yes", "onshape-mcp"]
    }
  },
  "plugin": ["@onshape-mcp/opencode-auth"]
}
```

Then run `opencode auth login` to complete the OAuth flow.

### Other MCP Clients

Any MCP client that supports stdio transport can launch this server. The command is `npx --yes onshape-mcp`. Pass credentials as environment variables (see [Authentication](#authentication)).

## Authentication

The server supports two authentication methods. It auto-detects which to use based on available credentials.

> **Security note:** Prefer environment variables over config files for credentials. If your MCP client supports setting environment variables (most do), use that. Avoid putting secrets in config files where possible — they're harder to keep out of version control and backups.

### API Keys (Basic Auth)

The simplest option. Create an API key pair and pass them as environment variables.

1. Go to the [Onshape Developer Portal](https://dev-portal.onshape.com/keys) and create an API key pair.
2. Set the keys as environment variables in your MCP client config (see the [client configuration examples](#mcp-client-configuration) above).

| Variable | Description |
| -------- | ----------- |
| `ONSHAPE_MCP_AUTH__ACCESS_KEY` | Your Onshape API access key |
| `ONSHAPE_MCP_AUTH__SECRET_KEY` | Your Onshape API secret key |

### OAuth 2.0

OAuth provides scoped, revocable access and is the better choice for ongoing use. It requires registering an "OAuth application" in Onshape's developer portal — this is just creating a set of OAuth credentials, not writing any code or building a project.

#### 1. Create an Onshape OAuth application

1. Go to [cad.onshape.com/appstore/dev-portal](https://cad.onshape.com/appstore/dev-portal) and sign in.
2. Click **OAuth applications** in the left sidebar.
3. Click **Create new OAuth application**.
4. Fill out the form:
   - **Name:** Anything you like (e.g. "My MCP Server")
   - **Primary format:** A unique identifier (e.g. "com.yourname.onshape-mcp") — cannot be changed later
   - **Summary:** Brief description (e.g. "MCP server for AI-assisted CAD")
   - **Redirect URLs:** `http://localhost:18338/callback`
   - **OAuth URL:** Leave blank
   - **Permissions:** Check at minimum **Application can read your documents** (`OAuth2Read`). Add **Application can write to your documents** (`OAuth2Write`) if you plan to use `modify` mode, and **Application can delete documents and workspaces** (`OAuth2Delete`) for `destroy` mode.
5. Click **Create application**.
6. **Copy the OAuth secret from the popup** — you will not be able to see it again.
7. Copy the **OAuth client identifier** from the app details page.

#### 2. Configure the MCP server

**With OpenCode (recommended):** Add the `@onshape-mcp/opencode-auth` plugin to your `opencode.json` (see the [OpenCode config example](#opencode) above), then run `opencode auth login`. The plugin will prompt for your client ID and secret, open a browser for authorization, and handle the rest.

**With other MCP clients:** Pass the client credentials as environment variables and complete the OAuth flow manually:

| Variable | Description |
| -------- | ----------- |
| `ONSHAPE_MCP_AUTH__CLIENT_ID` | OAuth client identifier from step 7 above |
| `ONSHAPE_MCP_AUTH__CLIENT_SECRET` | OAuth secret from step 6 above |

See the [Authentication docs](docs/src/project/authentication.md) for full details on token storage, refresh behavior, and security.

## Tools

### API Discovery

| Tool | Description |
| ---- | ----------- |
| `onshape_api_search` | Find Onshape API endpoints by keyword, HTTP method, or tag |
| `onshape_api_explain` | Get full parameter schemas and docs for a specific endpoint |
| `onshape_api_call` | Invoke an endpoint with path/query/body params and file references |
| `onshape_api_schema` | Look up Onshape API schema definitions by name |

### Auth Management

| Tool | Description |
| ---- | ----------- |
| `onshape_auth_status` | Check current auth state (valid, invalid, expired, not configured) |
| `onshape_auth_login` | Start an OAuth 2.0 authorization flow via browser |

### CAD Inspection

| Tool | Description |
| ---- | ----------- |
| `onshape_screenshot` | Render a Part Studio view (preset or custom angles) and save as PNG |
| `onshape_error_lookup` | Resolve FeatureScript error enum values to human-readable messages |

### Knowledge & Onboarding

| Tool | Description |
| ---- | ----------- |
| `onshape_mcp_get_started` | Essential guidance for working with this server |
| `onshape_list_resources` | List available insight documents |
| `onshape_read_resource` | Read a specific insight document by URI |

### Typical Workflow

The AI uses the API discovery tools in a natural progression — you just describe what you want in natural language:

**1. Search** for relevant endpoints:

```json
onshape_api_search({ "query": "get documents", "method": "GET" })
```

**2. Explain** a specific endpoint to learn its parameters:

```json
onshape_api_explain({ "endpoint": "getDocuments" })
```

**3. Call** the endpoint:

```json
onshape_api_call({
  "endpoint": "getDocuments",
  "query_params": { "q": "robot arm", "limit": "5" }
})
```

## Resources

The server ships with built-in insight documents covering Onshape CAD concepts — sketches, extrudes, revolves, sweeps, fillets, construction planes, shaded views, FeatureScript, and more. AI assistants can browse these via `onshape_list_resources` / `onshape_read_resource` to learn tested patterns and working examples without needing external documentation.

## Permission Model

> **Note:** Permission modes are planned but not yet enforced. Currently all HTTP methods that the server knows about are available. See the [implementation roadmap](docs/src/project/implementation.md) for status.

Three permission modes will control which HTTP methods are allowed:

| Mode | Allowed Methods | Use Case |
| ---- | --------------- | -------- |
| `read` | GET | Safe exploration and export |
| `modify` | GET, POST, PUT, PATCH | Creating and editing |
| `destroy` | All including DELETE | Full access |

These will be configurable via `mode.max`, `mode.initial`, and `mode.allow_escalation`. See [Configuration docs](docs/src/project/configuration.md) for details.

## Configuration

Settings are loaded with this precedence (highest wins): CLI flags > environment variables > config file > defaults.

| Setting | Env Var | Default |
| ------- | ------- | ------- |
| Access Key | `ONSHAPE_MCP_AUTH__ACCESS_KEY` | — |
| Secret Key | `ONSHAPE_MCP_AUTH__SECRET_KEY` | — |
| Auth Method | `ONSHAPE_MCP_AUTH__METHOD` | `auto` |
| API Timeout | `ONSHAPE_MCP_API__TIMEOUT` | `30s` |

See the [full settings reference](docs/src/project/configuration.md#all-settings-reference) for all options including config file format and locations.

## Direct Installation

Most users won't need this — your MCP client handles launching the server via `npx`. But if you want to run the binary directly:

```sh
# Via cargo
cargo install onshape-mcp

# Pre-built binaries
# Download from https://github.com/altendky/onshape-mcp/releases
```

## Documentation

Full project documentation is in [`docs/src/project/`](docs/src/project/index.md), covering:

- [Architecture](docs/src/project/architecture.md) — Sans-IO design, crate structure
- [MCP Tools](docs/src/project/mcp-tools.md) — Tool specs, permission model
- [Configuration](docs/src/project/configuration.md) — All settings, config file format
- [Authentication](docs/src/project/authentication.md) — Auth methods, OAuth flow, security
- [Development](docs/src/project/development.md) — Local setup, testing, pre-commit hooks
- [Contributing](docs/src/project/contributing.md) — Contribution guidelines

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
