# OAuth Token Exchange Proxy

A Cloudflare Worker that can be self-hosted as an OAuth2 token exchange proxy for Onshape.
The worker holds the OAuth2 client secret and exposes endpoints that CLI applications can call to exchange authorization codes for tokens and to refresh tokens — without the CLI ever needing the client secret.

The repository provides software, not a public proxy service. Direct OAuth is
the recommended default. Proxy users must deploy this worker themselves and
explicitly supply its URL, represented here as
`https://oauth-proxy.example.com`.

## Motivation

The MCP server's CLI uses Onshape's OAuth2 Authorization Code flow.
Without the proxy, the CLI must hold both the `client_id` (public) and `client_secret` (sensitive) directly — either in configuration files or embedded in the binary.
Distributing the client secret to every CLI installation is a security concern: if the secret is extracted, an attacker could impersonate the OAuth application.

The proxy centralizes the client secret in a single controlled location.
The CLI continues to handle the browser-based authorization flow (constructing the authorize URL, running a localhost callback server to receive the authorization code) but delegates the two operations that require the client secret to the proxy:

1. Exchanging an authorization code for access + refresh tokens
2. Refreshing an expired access token

## Architecture

The proxy is a thin HTTP forwarder.
It receives requests from the CLI, adds the `client_id` and `client_secret` from its environment, constructs a form-encoded POST to Onshape's token endpoint, and returns the response as-is.
The proxy does not parse, interpret, or store token responses.

```text
┌──────────┐         ┌──────────────────────┐         ┌─────────────────────┐
│          │  POST   │                      │  POST   │                     │
│   CLI    │────────>│  Cloudflare Worker   │────────>│  Onshape OAuth      │
│          │  JSON   │  (adds client_id &   │  form   │  Token Endpoint     │
│          │<────────│   client_secret)     │<────────│                     │
│          │  JSON   │                      │  JSON   │                     │
└──────────┘         └──────────────────────┘         └─────────────────────┘
                     oauth-proxy.example.com                          oauth.onshape.com
```

### Technology Choice

The worker is implemented in TypeScript, not Rust.
See [Decisions](decisions.md#oauth-proxy-language) for the rationale.

### Project Structure

The worker lives in `workers/oauth-proxy/` in the repository root, separate from the `npm/` directory which holds publishable npm packages.

```text
workers/oauth-proxy/
├── src/
│   ├── index.ts          # I/O entry point (thin executor)
│   ├── handler.ts        # Pure request handler (no I/O, fully testable)
│   ├── types.ts          # Shared types (Effect, Env, RequestContext)
│   └── dns.ts            # DNS-over-HTTPS hostname resolution
├── test/
│   └── handler.test.ts   # Tests for pure handler logic (no mocking)
├── wrangler.toml          # Worker config (name, routes, compatibility date)
├── package.json           # Dependencies, dev/deploy scripts
├── tsconfig.json          # TypeScript config
└── vitest.config.ts       # Test configuration
```

The worker follows a sans-IO pattern: `handler.ts` contains all routing, validation, and request-construction logic as pure functions that return `Effect` descriptors.
`index.ts` is a thin executor (~30 lines) that parses the request, resolves DNS, calls the handler, and executes the returned effect.
Tests import only `handler.ts` and assert on returned effects — no fetch mocking needed.

## Endpoints

### `GET /config`

Returns the proxy's OAuth client configuration.
IP-restricted like all endpoints except `GET /health`. The client ID itself is
public, but restricting `/config` lets clients fail before starting a browser
flow from an unauthorized network.

**Response:**

```json
{
  "client_id": "<ONSHAPE_CLIENT_ID>"
}
```

This allows CLI tools to discover the client ID after the user explicitly
supplies the self-hosted proxy URL.

### `POST /token/exchange`

Exchange an authorization code for access and refresh tokens.

**Request:**

```json
{
  "code": "<authorization_code>",
  "redirect_uri": "<redirect_uri_used_in_authorize_step>",
  "code_verifier": "<pkce_code_verifier>"
}
```

The `code_verifier` field is required when the authorization request used PKCE (`code_challenge` + `code_challenge_method`), which is the standard flow.

**Worker behavior:**

1. Validates request body (returns 400 if malformed)
2. Constructs form-encoded body: `grant_type=authorization_code&code=<code>&client_id=<id>&client_secret=<secret>&redirect_uri=<redirect_uri>&code_verifier=<code_verifier>`
3. POSTs to `https://oauth.onshape.com/oauth/token`
4. Returns Onshape's response (status code and body) as-is

**Success response** (proxied from Onshape):

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "token_type": "bearer",
  "expires_in": 3600,
  "scope": "OAuth2Read OAuth2Write"
}
```

### `POST /token/refresh`

Refresh an expired access token.

**Request:**

```json
{
  "refresh_token": "<refresh_token>"
}
```

**Worker behavior:**

1. Validates request body (returns 400 if malformed)
2. Constructs form-encoded body: `grant_type=refresh_token&refresh_token=<token>&client_id=<id>&client_secret=<secret>`
3. POSTs to `https://oauth.onshape.com/oauth/token`
4. Returns Onshape's response as-is

### `GET /health`

Health check endpoint.
Returns `{ "status": "ok" }` with status 200.
Not IP-restricted — useful for external monitoring.

### Error Responses

| Condition | Status | Body |
| --------- | ------ | ---- |
| Disallowed source IP | 403 | `{ "error": "forbidden", "source_ip": "<ip>" }` |
| Malformed or missing JSON body | 400 | `{ "error": "<descriptive message>" }` |
| Unknown route | 404 | `{ "error": "not_found" }` |
| Wrong HTTP method | 405 | `{ "error": "method_not_allowed" }` |
| Upstream Onshape error | Forwarded | Forwarded as-is |
| `ALLOWED_SOURCES` not configured | 500 | `{ "error": "server misconfigured" }` |

The `client_secret` is never included in any response or error message.

## Access Restriction

Access is restricted by source IP.
The allowed sources are configured via the `ALLOWED_SOURCES` environment variable, set in the Cloudflare dashboard (not committed to the repository).

### Configuration

`ALLOWED_SOURCES` is a comma-separated list of IP addresses and/or hostnames.
Each entry is auto-detected:

- If the entry parses as an IP address, it is compared directly against the `CF-Connecting-IP` request header.
- Otherwise, it is treated as a hostname and resolved at request time via Cloudflare's DNS-over-HTTPS API (`https://cloudflare-dns.com/dns-query`).
  Both A (IPv4) and AAAA (IPv6) records are queried.

Example: `ALLOWED_SOURCES=home.example.com,203.0.113.5`

Configure the public egress IPv4 and/or IPv6 address that Cloudflare reports for each permitted network, or a hostname that resolves to it. `203.0.113.5` is a documentation-only address and must be replaced.

This handles dynamic DNS automatically — if the IP for `home.example.com` changes, the worker resolves the new IP on the next request.
The DNS-over-HTTPS lookup adds approximately 5–20ms of latency (Cloudflare-internal).

### Automatic IPv4 Retry

On dual-stack networks the OS may prefer IPv6, causing the proxy to see an IPv6 source IP even when `ALLOWED_SOURCES` only contains IPv4 addresses (or hostnames that resolve to IPv4).
Both the Rust MCP server and the OpenCode auth plugin detect this situation and retry with the other address family automatically.

**Rust MCP server** (token refresh):

1. Sends the refresh request with the default HTTP client.
2. If the proxy returns 403 with an IPv6 `source_ip`, builds a new `reqwest::Client` with `local_address` set to `0.0.0.0` (`Ipv4Addr::UNSPECIFIED`) and retries.

**OpenCode auth plugin** (token exchange):

Bun's `fetch` ignores both `localAddress` (undici) and `family: 4` (node:https), so the plugin resolves the proxy hostname to all IPv6 (AAAA) and IPv4 (A) records via `dns.resolve6` / `dns.resolve4` and connects to each resolved IP directly using `node:https` with `servername` for TLS SNI and `Host` for HTTP routing.

1. Resolves the proxy hostname to all AAAA and A records in parallel.
2. Tries each IPv6 address — within a family, only connection errors advance to the next address; any HTTP response (even 403) is a definitive answer.
3. If IPv6 yields 403 or all IPv6 addresses fail to connect, tries each IPv4 address the same way.
4. If both families fail with 403, the error message includes all `source_ip` values seen, giving the user the information needed to update `ALLOWED_SOURCES`.
5. If every address in both families fails to connect, reports the total number of addresses attempted.

This is transparent to the user — no manual IP configuration is needed as long as one of IPv4 or IPv6 is allowed.

### Exempt Endpoints

`GET /health` is not IP-restricted, allowing external monitoring without whitelisting the monitoring service's IP.

### Updating the Whitelist

To add or remove allowed sources:

1. Go to the Cloudflare dashboard
2. Navigate to Workers & Pages > `onshape-oauth-proxy` > Settings > Variables
3. Edit the `ALLOWED_SOURCES` variable
4. Save — changes take effect immediately (Workers are stateless, no restart needed)

## Secrets Management

| Item | Storage | How to Set |
| ---- | ------- | --------- |
| `ONSHAPE_CLIENT_ID` | Cloudflare encrypted secret | `wrangler secret put ONSHAPE_CLIENT_ID` |
| `ONSHAPE_CLIENT_SECRET` | Cloudflare encrypted secret | `wrangler secret put ONSHAPE_CLIENT_SECRET` |
| `ALLOWED_SOURCES` | Cloudflare dashboard env var | Dashboard > Worker > Settings > Variables |

None of these values appear in `wrangler.toml` or any committed file.
The `ONSHAPE_CLIENT_ID` is stored as a secret (rather than a plain variable) for consistency, even though it is not sensitive on its own.

## CLI Integration Flow

The OpenCode auth plugin handles the browser-based authorization flow and delegates token operations to the proxy.
The plugin still runs a localhost callback server and handles PKCE.

1. Plugin fetches the proxy's client ID:

   ```text
   GET https://oauth-proxy.example.com/config
   → { "client_id": "<ID>" }
   ```

2. Plugin constructs the Onshape authorization URL using the client ID:

   ```text
   https://oauth.onshape.com/oauth/authorize?client_id=<ID>&redirect_uri=http://localhost:<PORT>/callback&response_type=code&state=<STATE>&code_challenge=<CHALLENGE>&code_challenge_method=S256
   ```

3. Plugin opens the user's browser to this URL
4. User logs into Onshape and authorizes the application
5. Onshape redirects the browser to `http://localhost:<PORT>/callback?code=<CODE>&state=<STATE>`
6. Plugin's local callback server captures the authorization code
7. Plugin POSTs to the proxy:

   ```text
   POST https://oauth-proxy.example.com/token/exchange
   Content-Type: application/json

   { "code": "<CODE>", "redirect_uri": "http://localhost:<PORT>/callback", "code_verifier": "<VERIFIER>" }
   ```

8. Proxy returns the token response:

   ```json
   { "access_token": "...", "refresh_token": "...", "expires_in": 3600, ... }
   ```

   If the proxy returns 403, the plugin automatically retries with the other address family (see [Automatic IPv4 Retry](#automatic-ipv4-retry) above).
   If both families fail, the error message includes all source IPs that were tried.

9. Plugin saves tokens to the token file with `proxy_url` (not `client_secret`):

   ```json
   { "access_token": "...", "refresh_token": "...", "client_id": "...", "proxy_url": "https://oauth-proxy.example.com" }
   ```

10. When the access token expires, the MCP server refreshes via the proxy:

    ```text
    POST https://oauth-proxy.example.com/token/refresh
    Content-Type: application/json

    { "refresh_token": "<REFRESH_TOKEN>" }
    ```

The `redirect_uri` must be included in the exchange request because Onshape's token endpoint validates that it matches the URI used in the authorization step.

### Direct Mode (Recommended Default)

The plugin also supports a "direct" mode where the user provides both `client_id` and `client_secret`.
In this mode, the plugin exchanges tokens directly with Onshape (not via the proxy), and the token file includes `client_secret` instead of `proxy_url`.
The MCP server detects the mode from the token file and refreshes accordingly.

## Deployment

### Prerequisites

- Node.js (for wrangler)
- A Cloudflare account with a Workers subdomain or a domain you control
- The Onshape OAuth application's client ID and secret

### Initial Setup

```sh
cd workers/oauth-proxy
npm install
wrangler secret put ONSHAPE_CLIENT_ID
wrangler secret put ONSHAPE_CLIENT_SECRET
```

Configure `ALLOWED_SOURCES` in the Cloudflare dashboard after the first deploy.

The checked-in `wrangler.toml` uses the deployer's `workers.dev` subdomain and
does not assume a particular DNS zone or custom domain. Operators may configure
their own custom domain independently.

### Deploy

```sh
cd workers/oauth-proxy
npm run deploy
```

### Local Development

```sh
cd workers/oauth-proxy
npm run dev
```

This starts a local dev server.
To test with secrets locally, create a `.dev.vars` file (gitignored) with:

```text
ONSHAPE_CLIENT_ID=your-client-id
ONSHAPE_CLIENT_SECRET=your-client-secret
ALLOWED_SOURCES=127.0.0.1
```

### Verification

```sh
# Health check
curl https://oauth-proxy.example.com/health

# Token exchange (from an allowed source)
curl -X POST https://oauth-proxy.example.com/token/exchange \
  -H 'Content-Type: application/json' \
  -d '{"code": "test-code", "redirect_uri": "http://localhost:18338/callback"}'

# Token refresh (from an allowed source)
curl -X POST https://oauth-proxy.example.com/token/refresh \
  -H 'Content-Type: application/json' \
  -d '{"refresh_token": "test-refresh-token"}'
```

Run the disallowed-source check from a network whose public IPv4 or IPv6 egress address is not matched directly, or through a hostname, by `ALLOWED_SOURCES`. The expected response has HTTP status 403 and a body containing `{ "error": "forbidden", "source_ip": "..." }`.

```sh
# Verify 403 from a disallowed source
curl --include -X POST https://oauth-proxy.example.com/token/exchange \
  -H 'Content-Type: application/json' \
  -d '{"code": "test", "redirect_uri": "http://localhost:8080/callback"}'
```

## Security Considerations

### Trust Model

The proxy is a **trusted intermediary**.
It sees the token response from Onshape in memory during the proxied request/response cycle.
A compromised or malicious version of the worker could log or exfiltrate access tokens and refresh tokens from these responses, granting the ability to read and write Onshape data as the user.

This is inherent to the proxy design — the same trust model as any OAuth proxy.
The exposure is **transient**: tokens exist in worker memory only for the duration of a single request, and the worker has no persistent storage.

### Mitigations

- The worker code is auditable and minimal (~120 lines)
- Cloudflare Workers are isolated (no persistent filesystem, no ambient network access beyond explicit `fetch()` calls)
- The `client_secret` is stored as an encrypted Cloudflare secret, never in code
- IP restriction limits who can initiate token operations
- The worker is deployed from your own account — you control the code and deployment

### Tradeoff

This design trades **distributed secret** (every CLI installation holds the `client_secret`) for **centralized trust** (one server sees all token responses transiently).
Neither approach is strictly superior — the choice depends on the threat model.
For a personal or small-group tool where you control the server, the proxy approach is reasonable.

## Deferred: Server-Side Callback Flow

**Status**: Deferred.
Documented here for future consideration.

### Concept

Move the OAuth callback from the CLI's localhost server to the worker.
The CLI would call `POST /auth/start` (worker generates state + PKCE, returns authorize URL), open the browser, then poll `POST /auth/poll` to retrieve tokens.
This eliminates the localhost HTTP server, PKCE generation, and authorize URL construction from the CLI.

### What It Would Add

| Component | Detail |
| --------- | ------ |
| `POST /auth/start` | Generate state + PKCE, store in KV, return authorize URL |
| `GET /callback` | Receive Onshape redirect, exchange code for tokens, store tokens in KV |
| `POST /auth/poll` | CLI retrieves tokens by state (one-time retrieval, then deleted) |
| Cloudflare KV namespace | Temporary storage for state/PKCE pairs and tokens |

Additional ~75 lines of worker code.

### What It Would Remove from the CLI

- Localhost HTTP callback server
- PKCE generation
- Authorize URL construction
- `redirect_uri` parameter management

### Infrastructure

Cloudflare KV free tier: 1,000 writes/day.
Each auth flow uses ~3 KV writes (store state+PKCE, store tokens, delete tokens).
Paid plan ($5/month): 1 million KV writes/month — sufficient for ~333,000 auth flows/month.

The Onshape OAuth application would need
`https://oauth-proxy.example.com/callback` registered as a redirect URI.

### Security Concern

**The server-side callback flow materially changes the worker's security posture.**

Without the callback flow (the current design), the worker sees tokens **transiently in memory** during the proxied response — they exist only for the duration of a single HTTP request and are never persisted.

With the callback flow, the worker:

- **Directly receives tokens** from Onshape (not just proxying a response)
- **Stores tokens in Cloudflare KV** for up to ~5 minutes (between the callback and the CLI's poll)
- Has a **wider window of exposure** during which tokens are persisted and retrievable

A compromised worker with the callback flow could:

- Log or exfiltrate access tokens and refresh tokens
- Read and write the user's Onshape data using the access token
- Maintain indefinite access via the refresh token (until the user manually revokes the OAuth app in Onshape's UI at cad.onshape.com > My Account > Applications > Revoke)

### Mitigations if Implemented

- Restrict `/auth/poll` to the IP address that called `/auth/start` (recorded at initiation time)
- One-time token retrieval: tokens deleted from KV after first successful poll
- Short TTL on KV entries (5 minutes)
- All endpoints except `/callback` remain IP-restricted via `ALLOWED_SOURCES`
- `ALLOWED_SOURCES` check applies to `/callback` as well (the user's browser typically shares the same public IP as the CLI behind NAT)

### Decision

The without-callback design has a meaningfully smaller attack surface.
The CLI simplification is valuable but does not justify the increased security exposure at this time.
If the callback flow is implemented in the future, the security considerations above must be addressed and documented prominently.
