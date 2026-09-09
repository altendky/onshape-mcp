# Enterprise split-server deployment

This deployment runs one hardened `onshape-mcp` instance on `10.254.50.30`.
The existing Caddy server on `10.254.50.70` terminates public HTTPS and proxies
requests to the MCP server over the private network. OAuth clients, MCP tokens,
and Onshape user tokens survive restarts in an AES-256-GCM encrypted state
file.

## Security model

- Every engineer authenticates with their own Onshape account through a
  company-owned Onshape OAuth application. The configured company ID binds the
  authorization flow to the intended Onshape enterprise.
- Any user who successfully completes the Onshape OAuth flow is accepted and
  has complete Onshape API capability, including create, update, and delete
  operations. There is no application-level user allowlist, so the server does
  not independently verify membership in the intended enterprise.
- MCP access and refresh tokens are audience-bound to this server's exact
  `/mcp` URL. PKCE S256 is mandatory for confidential and public clients.
- Durable OAuth state is encrypted at rest. The encryption key and Onshape
  client secret are mounted read-only and are never put in environment
  variables or command-line arguments.
- The application container is non-root, read-only, and capability-free. Port
  8080 is published only on the MCP server's private address. The MCP host
  firewall must accept it from `10.254.50.70` and reject all other sources.
- HTTP security headers, request-size limits, registration/pending-flow caps,
  and JSON audit events for Onshape API calls are enabled.

This design deliberately uses one application replica. The encrypted state
file is safe for restart durability, not concurrent writers on shared storage.
For active-active high availability, replace it with a transactional shared
state backend before adding replicas.

## Prerequisites

1. The company-managed MCP server at `10.254.50.30`, with Docker Engine and
   Docker Compose.
2. The Caddy server at `10.254.50.70`, reachable from the MCP server over the
   private network.
3. A public DNS record for the MCP hostname pointing to the Caddy server's
   public IP or public NAT address. Do not publish either `10.254.50.70` or
   `10.254.50.30` in public DNS; they are private addresses.
4. Public TCP 80/443 to Caddy, plus private TCP 8080 from `10.254.50.70` to
   `10.254.50.30`. Do not expose port 8080 to the internet.
5. Outbound HTTPS from the MCP server to `oauth.onshape.com` and
   `icepower.onshape.com`.
6. A company-owned Onshape OAuth application with this exact redirect URI:
   `https://<MCP_DOMAIN>/oauth/callback`.
7. The Onshape enterprise company ID configured for the integrated application.
8. Confirmation from the Onshape administrator that the OAuth application and
   enterprise policy permit only the accounts that should use this server.

Company-owned Onshape OAuth applications count against the company's API
limits. Confirm expected usage and annual limits with the Onshape administrator
before broad rollout.

## Configure

From the repository root:

```bash
cd deploy/enterprise
cp .env.example .env
mkdir -p secrets
openssl rand -base64 32 > secrets/state-encryption-key
printf '%s' 'replace-with-onshape-client-secret' > secrets/onshape-client-secret
sudo chown 10001:10001 secrets/state-encryption-key secrets/onshape-client-secret
sudo chmod 600 secrets/state-encryption-key secrets/onshape-client-secret
```

Keep `.env` and `secrets/` out of source control. Store copies of both secrets
in the company's secret manager. Losing the state encryption key makes the
persisted OAuth state unrecoverable; exposing it together with the state volume
exposes active credentials.

## Configure Caddy

The `Caddyfile` in this directory is intended for the separate Caddy server at
`10.254.50.70`. Merge its site block into that server's active Caddy
configuration. It routes the complete public origin—including `/mcp`, OAuth,
and discovery endpoints—to `http://10.254.50.30:8080`.

Configure the MCP server firewall to allow TCP port 8080 only from
`10.254.50.70`. Before testing the public hostname, run this from the Caddy
server:

```bash
curl --fail http://10.254.50.30:8080/ready
```

## Start and verify

```bash
docker compose up -d --build
docker compose ps
curl --fail http://10.254.50.30:8080/ready
curl --fail "https://${MCP_DOMAIN}/health"
curl --fail "https://${MCP_DOMAIN}/ready"
curl --fail "https://${MCP_DOMAIN}/.well-known/oauth-protected-resource/mcp"
```

The MCP client URL is `https://<MCP_DOMAIN>/mcp`. Each engineer completes the
browser OAuth flow once. The separate Caddy server preserves the public `Host`
header because the MCP transport rejects other authorities to prevent DNS
rebinding.

## Engineer onboarding (no terminal)

Engineers do not need the repository, configuration files, OAuth client ID, or
OAuth client secret. They only connect the MCP URL and sign in with Onshape.

### Claude

For a Claude Team or Enterprise organization, an Owner or Primary Owner adds
the connector once:

1. Open **Organization Settings → Connectors**.
2. Select **Add → Custom → Web**.
3. Enter `https://<MCP_DOMAIN>/mcp` and save it as `ICEpower Onshape`.

Each engineer then opens **Customize → Connectors**, finds `ICEpower Onshape`,
selects **Connect**, and completes the Onshape login. The server must be
publicly reachable over HTTPS because remote Claude connectors connect from
Anthropic's infrastructure. See [Claude's custom connector
guide](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp).

### Codex

Each engineer can connect entirely through the desktop interface:

1. Open **Settings → MCP servers → Add server**.
2. Enter `ICEpower Onshape` as the name.
3. Select **Streamable HTTP** and enter `https://<MCP_DOMAIN>/mcp`.
4. Save, restart when prompted, and select **Authenticate**.
5. Complete the Onshape login.

For a managed rollout, IT can distribute this Codex configuration so the MCP
server appears automatically and engineers only need to authenticate:

```toml
[mcp_servers.icepower_onshape]
url = "https://<MCP_DOMAIN>/mcp"
```

See the official [Codex MCP
guide](https://learn.chatgpt.com/docs/extend/mcp?surface=app) and [managed
configuration
guide](https://learn.chatgpt.com/docs/enterprise/managed-configuration).

## Operations

- Stream MCP logs with `docker compose logs -f onshape-mcp`; inspect Caddy logs
  on `10.254.50.70`. Application audit records use the
  event name `onshape_api_request` and include timestamp, Onshape user ID,
  method, path, outcome, and upstream status. They do
  not include request bodies, query parameters, OAuth tokens, or Authorization
  headers. Caddy redacts credential headers by default, and this deployment
  additionally redacts OAuth `code` and `state` query values from access logs.
- Back up the `oauth-state` Docker volume and the encryption key separately.
  Test restore procedures. A state backup without its matching key is unusable.
- Revoke OAuth access in Onshape when offboarding a user. Because the server has
  no local user allowlist, access control and revocation are managed through the
  Onshape OAuth application and the company's Onshape administration.
- Changing the state encryption key without re-encrypting the existing file
  intentionally fails startup. For simple key rotation, stop the service,
  archive the old state and key under the retention policy, generate a new key,
  remove the old state volume, and have engineers authorize again.
- Base images are digest-pinned. Patch the host and deliberately update those
  pins and rebuild regularly. Run `cargo deny check` and the full test suite in
  CI for every dependency or image update.

At the company edge, add per-IP rate limiting for `/oauth/register`,
`/oauth/authorize`, `/oauth/token`, and `/mcp`; alert on repeated 401/403/429
responses and registration-capacity exhaustion. If the service is internet
reachable, place it behind the company's WAF/DDoS service while preserving the
original `Host` header.

## Production acceptance checklist

- TLS certificate and callback URI match the configured public URL exactly.
- A company test account can complete OAuth and read and modify Onshape data.
- If access should be company-only, an account outside the intended enterprise
  cannot complete authorization under the configured Onshape policy.
- Restarting `onshape-mcp` preserves an existing client's refresh flow.
- The encrypted state file contains no recognizable token plaintext and is mode
  0600 inside the container.
- Backups restore successfully on a disposable host.
- Audit logs reach the company SIEM with an agreed retention period.
- Alerts cover readiness failures, restart loops, abnormal 4xx/5xx rates, and
  Onshape API-limit responses.
