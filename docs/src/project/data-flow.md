# Data Flow

This page contains sequence diagrams and state diagrams for the three operating
modes and their authentication lifecycles. For configuration details see
[Configuration](configuration.md); for auth credential sources see
[Authentication](authentication.md); for the OAuth proxy itself see
[OAuth Proxy](oauth-proxy.md).

## Operating Modes Overview

The MCP server has three operating modes. **Stdio** is the default for local
single-user use (Claude Desktop, OpenCode, etc.). **Streamable HTTP** is for
experimental, independently self-hosted remote multi-user deployments. No
public service is provided, and ChatGPT currently fails as tracked in
[#546](https://github.com/altendky/onshape-mcp/issues/546). **CLI Auth Login**
is a standalone command that completes the OAuth flow and writes a token file.

```mermaid
flowchart TB
    subgraph stdio ["Stdio Mode (local, single-user)"]
        MC1[MCP Client<br>Claude Desktop / OpenCode]
        MCP1[onshape-mcp<br>stdio transport]
        MC1 <-->|stdin / stdout<br>MCP protocol| MCP1
    end

    subgraph http ["Streamable HTTP Mode (remote, multi-user)"]
        MC2[MCP Client<br>Claude.ai / remote client]
        MCP2[onshape-mcp http<br>axum server]
        MC2 <-->|HTTPS<br>Streamable HTTP| MCP2
    end

    subgraph cli ["CLI Auth Login (standalone)"]
        CLI[onshape-mcp auth login]
        Browser[User's Browser]
        CLI -->|opens| Browser
    end

    Proxy[OAuth Proxy<br>Cloudflare Worker]
    Onshape[Onshape API<br>cad.onshape.com]
    OnshapeOAuth[Onshape OAuth<br>oauth.onshape.com]
    TokenFile[(tokens.json)]

    MCP1 -->|Basic or Bearer auth| Onshape
    MCP1 -.->|optional explicit<br>proxy refresh| Proxy
    MCP1 -.->|token refresh<br>direct only| OnshapeOAuth
    MCP1 <-.->|file watcher| TokenFile

    MCP2 -->|per-user Bearer auth| Onshape
    MCP2 <-->|server-side OAuth| OnshapeOAuth

    CLI -.->|explicit self-hosted<br>proxy mode| Proxy
    CLI -->|direct mode<br>default| OnshapeOAuth
    Proxy -->|forwards with<br>client_secret| OnshapeOAuth
    CLI -->|save tokens| TokenFile
    Browser <-->|authorize + callback| OnshapeOAuth

    style stdio fill:#e8f4e8,stroke:#2d7d2d
    style http fill:#e8e8f4,stroke:#2d2d7d
    style cli fill:#f4e8e8,stroke:#7d2d2d
```

---

## Stdio Mode

### API Key Request Flow

When using API key (Basic) authentication, every request includes a static
`Authorization: Basic <base64(access_key:secret_key)>` header. There is no
token lifecycle — credentials are validated on first use.

```mermaid
sequenceDiagram
    participant Client as MCP Client
    participant MCP as onshape-mcp<br>(stdio)
    participant Onshape as Onshape API

    Client->>MCP: call_tool (stdin)
    MCP->>MCP: onshape-mcp-core::call_tool()
    MCP->>Onshape: GET/POST /api/v14/...<br>Authorization: Basic <credentials>
    Onshape-->>MCP: 200 OK + response body
    MCP-->>Client: tool result (stdout)
```

### OAuth Login — Self-Hosted Proxy Mode

Optional explicit login flow. The CLI never sees the OAuth client secret — the proxy
adds it when forwarding to Onshape. The token file stores `proxy_url` so the
server knows to use the proxy for future refreshes.

```mermaid
sequenceDiagram
    participant CLI as onshape-mcp<br>auth login
    participant Proxy as OAuth Proxy<br>(Cloudflare Worker)
    participant Browser as User's Browser
    participant Onshape as Onshape OAuth<br>(oauth.onshape.com)
    participant File as tokens.json
    participant Watcher as File Watcher<br>(running MCP server)

    CLI->>Proxy: GET /config
    Proxy-->>CLI: { client_id }

    CLI->>CLI: Generate PKCE<br>(code_verifier + code_challenge)
    CLI->>CLI: Generate CSRF state

    CLI->>CLI: Start callback server<br>(localhost:18338)

    CLI->>Browser: Open authorize URL
    Browser->>Onshape: GET /oauth/authorize<br>?client_id=...&redirect_uri=<br>http://localhost:18338/callback<br>&code_challenge=...&state=...
    Note over Browser,Onshape: User logs in and authorizes

    Onshape-->>Browser: 302 Redirect
    Browser->>CLI: GET /callback?code=...&state=...

    CLI->>CLI: Validate CSRF state
    CLI->>CLI: Extract authorization code
    CLI->>CLI: Shut down callback server

    CLI->>Proxy: POST /token/exchange<br>{ code, redirect_uri,<br>  code_verifier }
    Proxy->>Proxy: Add client_id +<br>client_secret
    Proxy->>Onshape: POST /oauth/token<br>(form-encoded)
    Onshape-->>Proxy: { access_token, refresh_token,<br>  expires_in, ... }
    Proxy-->>CLI: Forward response as-is

    CLI->>File: Save tokens + client_id<br>+ proxy_url (0600 perms)

    File-->>Watcher: inotify / kqueue / poll<br>detects new file
    Watcher->>Watcher: Load tokens,<br>transition to OAuth state
```

### OAuth Login — Direct Mode

Default flow when the user provides both `client_id` and `client_secret`.
The CLI exchanges directly with Onshape — no proxy involved. The token file
stores `client_secret` so the server can refresh directly.

```mermaid
sequenceDiagram
    participant CLI as onshape-mcp<br>auth login
    participant Browser as User's Browser
    participant Onshape as Onshape OAuth<br>(oauth.onshape.com)
    participant File as tokens.json
    participant Watcher as File Watcher<br>(running MCP server)

    CLI->>CLI: Generate PKCE<br>(code_verifier + code_challenge)
    CLI->>CLI: Generate CSRF state

    CLI->>CLI: Start callback server<br>(localhost:18338)

    CLI->>Browser: Open authorize URL
    Browser->>Onshape: GET /oauth/authorize<br>?client_id=...&redirect_uri=<br>http://localhost:18338/callback<br>&code_challenge=...&state=...
    Note over Browser,Onshape: User logs in and authorizes

    Onshape-->>Browser: 302 Redirect
    Browser->>CLI: GET /callback?code=...&state=...

    CLI->>CLI: Validate CSRF state
    CLI->>CLI: Extract authorization code
    CLI->>CLI: Shut down callback server

    CLI->>Onshape: POST /oauth/token<br>(form-encoded: code, client_id,<br> client_secret, code_verifier,<br> redirect_uri)
    Onshape-->>CLI: { access_token, refresh_token,<br>  expires_in, ... }

    CLI->>File: Save tokens + client_id<br>+ client_secret (0600 perms)

    File-->>Watcher: inotify / kqueue / poll<br>detects new file
    Watcher->>Watcher: Load tokens,<br>transition to OAuth state
```

### OAuth Steady-State Request with Token Refresh

Each API request goes through a two-phase check: **proactive** (before the
request, if the token expires within 60 seconds) and **reactive** (after the
request, if Onshape returns 401). At most one refresh-and-retry occurs per
request.

```mermaid
sequenceDiagram
    participant Client as MCP Client
    participant MCP as onshape-mcp
    participant Refresh as Refresh Target<br>(proxy or Onshape)
    participant Onshape as Onshape API
    participant File as tokens.json

    Client->>MCP: call_tool (stdin)

    MCP->>MCP: pre_execute_action()<br>Token expires in < 60s?

    alt Token expiring soon (proactive refresh)
        MCP->>Refresh: POST /token/refresh<br>{ refresh_token }
        Refresh-->>MCP: { access_token, refresh_token,<br>  expires_in, ... }
        MCP->>File: Save refreshed tokens
        MCP->>MCP: Rebuild HTTP client<br>with new token
    end

    MCP->>Onshape: API request<br>Authorization: Bearer <token>

    alt 200 OK
        Onshape-->>MCP: Response body
    else 401 Unauthorized (reactive refresh)
        Onshape-->>MCP: 401
        MCP->>MCP: post_execute_action()<br>Already refreshed?

        alt Not yet refreshed
            MCP->>Refresh: POST /token/refresh<br>{ refresh_token }

            alt Refresh succeeds
                Refresh-->>MCP: New tokens
                MCP->>File: Save refreshed tokens
                MCP->>MCP: Rebuild HTTP client
                MCP->>Onshape: Retry API request
                Onshape-->>MCP: Response
            else Permanent failure (invalid_grant)
                Refresh-->>MCP: Error
                MCP->>MCP: Transition to<br>OAuthPending state
                MCP-->>Client: Error: re-authorization required
            end
        else Already refreshed this request
            MCP-->>Client: Error: unauthorized
        end
    end

    MCP-->>Client: tool result (stdout)
```

### Authentication State Machine

The stdio server maintains an `ApiState` that governs which operations are
available. The file watcher and refresh outcomes drive transitions. The
`Basic` state is static (no transitions). `HttpOAuth` (used in HTTP mode)
is per-request and does not participate in this state machine.

```mermaid
stateDiagram-v2
    [*] --> NotConfigured : No credentials found
    [*] --> OAuthPending : Client credentials present,<br>no token file
    [*] --> Basic : access_key + secret_key present
    [*] --> OAuth : Client credentials +<br>token file present

    NotConfigured --> OAuth : File watcher detects token file<br>with embedded client credentials

    OAuthPending --> OAuth : File watcher detects<br>token file appears

    OAuth --> OAuth : File watcher detects<br>externally refreshed tokens
    OAuth --> OAuth : Proactive or reactive<br>token refresh succeeds
    OAuth --> OAuthPending : Permanent refresh failure<br>(invalid_grant /<br>unauthorized_client)

    OAuthPending --> OAuth : User completes re-authorization<br>(file watcher detects new tokens)

    state NotConfigured {
        [*] : No complete credential set
    }
    state OAuthPending {
        [*] : Watching for token file
    }
    state Basic {
        [*] : Static API key auth
    }
    state OAuth {
        [*] : Active token management
    }
```

---

## Streamable HTTP Mode

### Client Onboarding

Remote MCP clients (e.g. Claude.ai) discover the server's OAuth metadata,
register dynamically, and go through a double OAuth flow: the MCP client
authenticates to the MCP server, and the MCP server authenticates to
Onshape on behalf of the user. The server operator's Onshape OAuth app
credentials are used for the Onshape leg.

This mode does not use the local OAuth proxy. It is experimental, is not
broadly client-verified, and is operated only by independent self-hosters.

```mermaid
sequenceDiagram
    participant Client as MCP Client<br>(Claude.ai)
    participant Server as onshape-mcp http<br>(axum)
    participant Browser as User's Browser
    participant Onshape as Onshape OAuth<br>(oauth.onshape.com)

    Note over Client,Server: Discovery
    Client->>Server: GET /.well-known/<br>oauth-authorization-server
    Server-->>Client: { authorization_endpoint,<br>  token_endpoint,<br>  registration_endpoint, ... }

    Note over Client,Server: Dynamic Client Registration
    Client->>Server: POST /oauth/register<br>{ redirect_uris, ... }
    Server-->>Client: { client_id, client_secret,<br>  redirect_uris }

    Note over Client,Server: Authorization (MCP leg)
    Client->>Client: Generate PKCE
    Client->>Server: GET /oauth/authorize<br>?client_id=...&redirect_uri=...<br>&code_challenge=...&state=...

    Note over Server,Onshape: Authorization (Onshape leg)
    Server->>Server: Generate own PKCE +<br>CSRF for Onshape leg
    Server->>Server: Store PendingAuth
    Server-->>Browser: 302 Redirect to Onshape<br>oauth.onshape.com/oauth/authorize<br>?client_id=<operator's ID><br>&redirect_uri=<server>/oauth/callback<br>&code_challenge=<server's challenge>

    Note over Browser,Onshape: User authorizes
    Browser->>Onshape: User logs in, authorizes app
    Onshape-->>Browser: 302 Redirect
    Browser->>Server: GET /oauth/callback<br>?code=<onshape_code>&state=...

    Note over Server,Onshape: Token exchange (Onshape)
    Server->>Server: Validate CSRF, consume PendingAuth
    Server->>Onshape: POST /oauth/token<br>(exchange onshape_code +<br>server's PKCE verifier)
    Onshape-->>Server: { access_token, refresh_token,<br>  expires_in }

    Note over Server: User verification
    Server->>Onshape: GET /api/v10/users/sessioninfo<br>Authorization: Bearer <onshape_token>
    Onshape-->>Server: { id: "<user_id>", ... }
    Server->>Server: Check user_id ∈ allowed_users

    Note over Server: Issue MCP credentials
    Server->>Server: Store Onshape tokens<br>keyed by user_id
    Server->>Server: Generate MCP auth code
    Server-->>Client: 302 Redirect to client's<br>redirect_uri?code=<mcp_code>&state=...

    Note over Client,Server: Token exchange (MCP)
    Client->>Server: POST /oauth/token<br>{ grant_type: authorization_code,<br>  code: <mcp_code>,<br>  code_verifier: <client's verifier> }
    Server-->>Client: { access_token: <mcp_token>,<br>  refresh_token: <mcp_refresh>,<br>  expires_in: 3600 }
```

### Steady-State Request

Once onboarded, the MCP client includes its MCP bearer token on every
request to `/mcp`. The auth middleware validates the token and injects a
`UserContext` containing the user's Onshape credentials. The request handler
builds a per-user Onshape HTTP client for the API call.

```mermaid
sequenceDiagram
    participant Client as MCP Client
    participant MW as Auth Middleware
    participant Handler as MCP Request Handler
    participant Onshape as Onshape API

    Client->>MW: POST /mcp<br>Authorization: Bearer <mcp_token><br>{ tool call }

    MW->>MW: Look up mcp_token<br>in issued tokens
    MW->>MW: Check not expired

    alt Token valid
        MW->>MW: Look up user's Onshape tokens<br>by user_id
        MW->>Handler: Forward request +<br>UserContext { user_id,<br>onshape_tokens }
        Handler->>Handler: Build per-user<br>OnshapeClient
        Handler->>Handler: call_tool()
        Handler->>Onshape: API request<br>Authorization: Bearer<br><user's onshape_token>
        Onshape-->>Handler: Response
        Handler-->>Client: Tool result
    else Token expired or invalid
        MW-->>Client: 401 Unauthorized
    end
```

### Per-User Onshape Token Refresh

The HTTP server manages Onshape tokens for each user independently. A
per-user lock prevents concurrent refresh races when multiple requests
arrive simultaneously for the same user. The operator's Onshape OAuth app
credentials are used for all refreshes.

```mermaid
sequenceDiagram
    participant Req1 as Request 1<br>(user A)
    participant Req2 as Request 2<br>(user A)
    participant Lock as Per-User Lock<br>(user A)
    participant Server as OAuthServerState
    participant Onshape as Onshape OAuth

    Note over Req1,Req2: Two concurrent requests<br>for the same user

    Req1->>Server: Check token expiry<br>for user A
    Server->>Server: Token expiring within 60s

    Req1->>Lock: Acquire refresh lock

    Req2->>Server: Check token expiry<br>for user A
    Server->>Server: Token expiring within 60s
    Req2->>Lock: Acquire refresh lock<br>(blocks — Req1 holds it)

    Req1->>Onshape: POST /oauth/token<br>{ grant_type: refresh_token,<br>  refresh_token: <user A's token>,<br>  client_id: <operator's ID>,<br>  client_secret: <operator's secret> }
    Onshape-->>Req1: { access_token, refresh_token,<br>  expires_in }
    Req1->>Server: Update user_tokens[user_A]
    Req1->>Lock: Release lock

    Lock-->>Req2: Lock acquired
    Req2->>Server: Re-check token expiry
    Note over Req2,Server: Token is now fresh<br>(Req1 already refreshed)
    Req2->>Lock: Release lock

    Req1->>Onshape: API call with new token
    Req2->>Onshape: API call with new token
```
