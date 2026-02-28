import type { Plugin } from "@opencode-ai/plugin";
import { mkdirSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { homedir } from "os";
import * as oauth from "oauth4webapi";
import {
  parseProxyError,
  resolveIpv4,
  resolveIpv6,
  tryAddresses,
} from "./ipv4-retry.js";

// Onshape OAuth server metadata (must match crates/onshape-client-core/src/oauth.rs)
const as: oauth.AuthorizationServer = {
  issuer: "https://oauth.onshape.com",
  authorization_endpoint: "https://oauth.onshape.com/oauth/authorize",
  token_endpoint: "https://oauth.onshape.com/oauth/token",
  code_challenge_methods_supported: ["S256"],
};
const OAUTH_SCOPES = "OAuth2Read OAuth2Write";

/** Default OAuth proxy URL. */
const DEFAULT_PROXY_URL = "https://onshape-oauth-proxy.fstab.workers.dev";

/**
 * Returns the data directory for onshape-mcp on the current platform.
 * Mirrors `default_token_file_path()` logic in onshape-client-core.
 */
function dataDir(): string {
  const platform = process.platform;
  const home = homedir();

  if (platform === "darwin") {
    return join(home, "Library", "Application Support", "onshape-mcp");
  } else if (platform === "win32") {
    return join(process.env.LOCALAPPDATA || join(home, "AppData", "Local"), "onshape-mcp");
  } else {
    // Linux and other Unix
    return join(process.env.XDG_DATA_HOME || join(home, ".local", "share"), "onshape-mcp");
  }
}

/**
 * Returns the default token file path for the current platform.
 */
function tokenFilePath(): string {
  return join(dataDir(), "tokens.json");
}

/**
 * Token file shape.  Either `client_secret` (direct mode) or `proxy_url`
 * (proxy mode) is set, never both.
 */
interface TokenFile {
  access_token: string;
  refresh_token: string;
  expires_at: string | null;
  token_type: string;
  scopes: string[] | null;
  client_id: string;
  client_secret?: string;
  proxy_url?: string;
}

/**
 * Save tokens to the token file with secure permissions.
 */
function saveTokens(tokens: TokenFile): void {
  const path = tokenFilePath();
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(tokens, null, 2), { mode: 0o600 });
}

// ============================================================================
// Shared helpers
// ============================================================================

/** Escape the five HTML-special characters to prevent injection. */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/**
 * Start a localhost HTTP server to receive the OAuth callback.
 * Returns the Bun server instance with a `_callbackUrl` property
 * that is set when the callback is received.
 */
function startCallbackServer() {
  const server = Bun.serve({
    port: 18338, // Fixed port to match Onshape OAuth app redirect URL registration
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/callback") {
        const error = url.searchParams.get("error");

        if (error) {
          (server as any)._callbackUrl = url;
          return new Response(
            `<html><body><h1>Authorization Failed</h1><p>${escapeHtml(error)}</p><p>You can close this window.</p></body></html>`,
            { headers: { "Content-Type": "text/html" } },
          );
        }

        (server as any)._callbackUrl = url;
        return new Response(
          `<html><body><h1>Authorization Successful</h1><p>You can close this window and return to your terminal.</p></body></html>`,
          { headers: { "Content-Type": "text/html" } },
        );
      }
      return new Response("Not found", { status: 404 });
    },
  });
  return server;
}

/**
 * Poll the callback server for the redirect URL.
 * Returns the URL once the callback is received, or undefined on timeout.
 */
async function pollForCallback(
  server: ReturnType<typeof startCallbackServer>,
  timeoutMs = 120_000,
): Promise<URL | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const callbackUrl: URL | undefined = (server as any)._callbackUrl;
    if (callbackUrl) return callbackUrl;
    await Bun.sleep(500);
  }
  return undefined;
}

export const OnshapeAuthPlugin: Plugin = async (_ctx) => {
  return {
    auth: {
      provider: "OnShape (onshape-mcp)",
      methods: [
        // ================================================================
        // Proxy mode: client secret held by the proxy server.
        // ================================================================
        {
          type: "oauth" as const,
          label: "Onshape OAuth (via proxy)",
          prompts: [
            {
              type: "text" as const,
              key: "proxy_url",
              message: "OAuth Proxy URL",
              placeholder: DEFAULT_PROXY_URL,
            },
          ],
          async authorize(inputs) {
            const proxyUrl = (inputs?.proxy_url || DEFAULT_PROXY_URL).replace(
              /\/$/,
              "",
            );

            // 1. Fetch client_id from the proxy's /config endpoint.
            const configResp = await fetch(`${proxyUrl}/config`);
            if (!configResp.ok) {
              throw new Error(
                `Failed to fetch proxy config: ${configResp.status} ${await configResp.text()}`,
              );
            }
            const proxyConfig = (await configResp.json()) as {
              client_id: string;
            };
            const clientId = proxyConfig.client_id;

            // 2. Preflight IP check — verify we can reach the proxy's
            //    IP-restricted endpoints before sending the user to the
            //    browser.  We POST a dummy body to /token/refresh: the
            //    proxy returns 400 (bad request) if our IP is allowed,
            //    or 403 if not.  This uses the same dual-stack address
            //    resolution as the real exchange request later.
            const proxyHostname = new URL(proxyUrl).hostname;
            const [ipv6Addrs, ipv4Addrs] = await Promise.all([
              resolveIpv6(proxyHostname),
              resolveIpv4(proxyHostname),
            ]);

            const preflightUrl = `${proxyUrl}/token/refresh`;
            const preflightBody = { refresh_token: "" };

            const triedSourceIps: string[] = [];
            const collect403Ip = (body: string) => {
              const err = parseProxyError(body);
              if (err?.source_ip) triedSourceIps.push(err.source_ip);
            };

            const ipv6Preflight = await tryAddresses(
              ipv6Addrs,
              preflightUrl,
              preflightBody,
            );

            const preflightPassed =
              ipv6Preflight && ipv6Preflight.status !== 403;

            if (!preflightPassed) {
              if (ipv6Preflight?.status === 403) {
                collect403Ip(ipv6Preflight.body);
              }

              const ipv4Preflight = await tryAddresses(
                ipv4Addrs,
                preflightUrl,
                preflightBody,
              );

              if (ipv4Preflight && ipv4Preflight.status !== 403) {
                // IPv4 is allowed — proceed (the exchange will also
                // need to use IPv4, which the exchange block handles).
              } else {
                if (ipv4Preflight?.status === 403) {
                  collect403Ip(ipv4Preflight.body);
                }
                const ipList = triedSourceIps.length > 0
                  ? ` Tried source IP${triedSourceIps.length > 1 ? "s" : ""}: ${triedSourceIps.join(", ")}.`
                  : "";
                const reason =
                  `Proxy rejected request (403 Forbidden).${ipList}`;

                // OpenCode's AuthOauthResult has no failure variant,
                // so we can't signal an error from authorize() directly.
                // Return a result whose instructions explain what went
                // wrong and whose callback immediately fails.
                return {
                  url: "-",
                  instructions: reason,
                  method: "auto" as const,
                  async callback() {
                    return { type: "failed" as const };
                  },
                };
              }
            }

            // 3. Generate PKCE verifier/challenge and CSRF state.
            const state = oauth.generateRandomState();
            const codeVerifier = oauth.generateRandomCodeVerifier();
            const codeChallenge =
              await oauth.calculatePKCECodeChallenge(codeVerifier);

            // 3. Start localhost callback server.
            const server = startCallbackServer();
            const port = server.port;
            const redirectUri = `http://localhost:${port}/callback`;

            // 4. Build the authorization URL.
            const authUrl = new URL(as.authorization_endpoint!);
            authUrl.searchParams.set("client_id", clientId);
            authUrl.searchParams.set("response_type", "code");
            authUrl.searchParams.set("redirect_uri", redirectUri);
            authUrl.searchParams.set("scope", OAUTH_SCOPES);
            authUrl.searchParams.set("state", state);
            authUrl.searchParams.set("code_challenge", codeChallenge);
            authUrl.searchParams.set("code_challenge_method", "S256");

            return {
              url: authUrl.toString(),
              instructions:
                "Open the URL above to authorize with Onshape. After granting access, you will be redirected back automatically.",
              method: "auto" as const,
              async callback() {
                try {
                  const callbackUrl = await pollForCallback(server);
                  if (!callbackUrl) return { type: "failed" as const };

                  // Validate the authorization response (state, error check).
                  const client: oauth.Client = { client_id: clientId };
                  const params = oauth.validateAuthResponse(
                    as,
                    client,
                    callbackUrl,
                    state,
                  );

                  // Extract the authorization code.
                  const code = new URL(callbackUrl.toString()).searchParams.get(
                    "code",
                  );
                  if (!code) {
                    throw new Error("No authorization code in callback URL");
                  }

                  // 5. Exchange via the proxy (not directly with Onshape).
                  //
                  // Why not just `fetch()`?
                  //
                  // The proxy restricts access by source IP via
                  // ALLOWED_SOURCES.  On dual-stack networks the OS
                  // typically prefers IPv6, so a plain `fetch()` may
                  // connect via IPv6 while ALLOWED_SOURCES only lists
                  // IPv4 addresses (or vice versa), resulting in 403.
                  //
                  // The Rust MCP server solves this with reqwest's
                  // `local_address(Ipv4Addr::UNSPECIFIED)` to force
                  // IPv4 on retry.  In Bun we have no equivalent:
                  //   - undici Agent `localAddress` — silently ignored
                  //   - node:https `family: 4`    — silently ignored
                  // Both are Bun compatibility gaps (as of Bun 1.3).
                  //
                  // So we do our own DNS resolution (AAAA + A records)
                  // and connect to each resolved IP directly via
                  // node:https with `servername` for TLS SNI and `Host`
                  // for HTTP routing.  We try IPv6 first (matching OS
                  // default preference), then fall back to IPv4 if we
                  // get 403 or all IPv6 addresses fail to connect.
                  // Within a family, only connection errors advance to
                  // the next address — any HTTP response (even 403) is
                  // a definitive answer from the proxy.
                  //
                  // See ipv4-retry.ts for the helpers.
                  const exchangeUrl = `${proxyUrl}/token/exchange`;
                  const exchangeBody = {
                    code,
                    redirect_uri: redirectUri,
                    code_verifier: codeVerifier,
                  };

                  const proxyHostname = new URL(exchangeUrl).hostname;
                  const [ipv6Addrs, ipv4Addrs] = await Promise.all([
                    resolveIpv6(proxyHostname),
                    resolveIpv4(proxyHostname),
                  ]);

                  const triedSourceIps: string[] = [];
                  let respStatus: number | undefined;
                  let respBody: string | undefined;

                  // Helper: record source_ip from a 403 response.
                  const collect403Ip = (body: string) => {
                    const err = parseProxyError(body);
                    if (err?.source_ip) triedSourceIps.push(err.source_ip);
                  };

                  // Try IPv6 addresses first (matches OS default preference).
                  const ipv6Result = await tryAddresses(
                    ipv6Addrs,
                    exchangeUrl,
                    exchangeBody,
                  );

                  if (ipv6Result && ipv6Result.status !== 403) {
                    // Got a non-403 response over IPv6 — use it.
                    respStatus = ipv6Result.status;
                    respBody = ipv6Result.body;
                  } else {
                    // Either all IPv6 addrs had connection errors, or
                    // we got 403.  Record the 403 IP and try IPv4.
                    if (ipv6Result?.status === 403) {
                      collect403Ip(ipv6Result.body);
                    }

                    const ipv4Result = await tryAddresses(
                      ipv4Addrs,
                      exchangeUrl,
                      exchangeBody,
                    );

                    if (ipv4Result) {
                      respStatus = ipv4Result.status;
                      respBody = ipv4Result.body;
                      if (ipv4Result.status === 403) {
                        collect403Ip(ipv4Result.body);
                      }
                    } else if (ipv6Result) {
                      // All IPv4 addrs had connection errors but we had
                      // an IPv6 response — use it (likely the 403).
                      respStatus = ipv6Result.status;
                      respBody = ipv6Result.body;
                    } else {
                      // Every address in both families failed to connect.
                      throw new Error(
                        `Failed to connect to proxy at ${proxyHostname} (tried ${ipv6Addrs.length} IPv6 and ${ipv4Addrs.length} IPv4 addresses)`,
                      );
                    }
                  }

                  if (respStatus !== 200) {
                    if (respStatus === 403) {
                      const ipList = triedSourceIps.length > 0
                        ? ` Tried source IP${triedSourceIps.length > 1 ? "s" : ""}: ${triedSourceIps.join(", ")}.`
                        : "";
                      throw new Error(
                        `Proxy rejected request (403 Forbidden).${ipList}`,
                      );
                    }
                    const proxyErr = parseProxyError(respBody!);
                    const detail = proxyErr
                      ? JSON.stringify(proxyErr)
                      : respBody;
                    throw new Error(
                      `Proxy token exchange failed (${respStatus}): ${detail}`,
                    );
                  }

                  const result = JSON.parse(respBody!) as {
                    access_token: string;
                    refresh_token?: string;
                    token_type?: string;
                    expires_in?: number;
                    scope?: string;
                  };

                  const expiresAt = result.expires_in
                    ? new Date(
                        Date.now() + result.expires_in * 1000,
                      ).toISOString()
                    : null;

                  const scopes = result.scope
                    ? result.scope.split(" ").filter(Boolean)
                    : null;

                  // 6. Save tokens with proxy_url (not client_secret).
                  saveTokens({
                    access_token: result.access_token,
                    refresh_token: result.refresh_token ?? "",
                    expires_at: expiresAt,
                    token_type: result.token_type || "bearer",
                    scopes,
                    client_id: clientId,
                    proxy_url: proxyUrl,
                  });

                  return {
                    type: "success" as const,
                    access: result.access_token,
                    refresh: result.refresh_token ?? "",
                    expires: result.expires_in ?? 3600,
                  };
                } catch (err) {
                  const msg = err instanceof Error ? err.message : String(err);
                  console.error(`Authorization failed: ${msg}`);
                  return { type: "failed" as const };
                } finally {
                  server.stop();
                }
              },
            };
          },
        },

        // ================================================================
        // Direct mode: client secret provided by the user.
        // ================================================================
        {
          type: "oauth" as const,
          label: "Onshape OAuth (direct)",
          prompts: [
            {
              type: "text" as const,
              key: "client_id",
              message: "Onshape OAuth Client ID",
              placeholder: "Enter your Onshape OAuth application client ID",
            },
            {
              type: "text" as const,
              key: "client_secret",
              message: "Onshape OAuth Client Secret",
              placeholder: "Enter your Onshape OAuth application client secret",
            },
          ],
          async authorize(inputs) {
            const clientId = inputs?.client_id;
            const clientSecret = inputs?.client_secret;

            if (!clientId || !clientSecret) {
              throw new Error("Client ID and Client Secret are required");
            }

            const client: oauth.Client = { client_id: clientId };
            const clientAuth = oauth.ClientSecretPost(clientSecret);

            // Generate CSRF state and PKCE verifier/challenge via oauth4webapi
            const state = oauth.generateRandomState();
            const codeVerifier = oauth.generateRandomCodeVerifier();
            const codeChallenge =
              await oauth.calculatePKCECodeChallenge(codeVerifier);

            // Start a local HTTP server for the OAuth callback
            const server = startCallbackServer();

            const port = server.port;
            const redirectUri = `http://localhost:${port}/callback`;

            // Build the authorization URL with CSRF state and PKCE challenge
            const authUrl = new URL(as.authorization_endpoint!);
            authUrl.searchParams.set("client_id", clientId);
            authUrl.searchParams.set("response_type", "code");
            authUrl.searchParams.set("redirect_uri", redirectUri);
            authUrl.searchParams.set("scope", OAUTH_SCOPES);
            authUrl.searchParams.set("state", state);
            authUrl.searchParams.set("code_challenge", codeChallenge);
            authUrl.searchParams.set("code_challenge_method", "S256");

            return {
              url: authUrl.toString(),
              instructions: "Open the URL above to authorize with Onshape. After granting access, you will be redirected back automatically.",
              method: "auto" as const,
              async callback() {
                try {
                  const callbackUrl = await pollForCallback(server);
                  if (!callbackUrl) return { type: "failed" as const };

                  // Validate the authorization response (checks state,
                  // detects OAuth errors) — throws on failure
                  const params = oauth.validateAuthResponse(
                    as,
                    client,
                    callbackUrl,
                    state,
                  );

                  // Exchange the authorization code for tokens
                  const response =
                    await oauth.authorizationCodeGrantRequest(
                      as,
                      client,
                      clientAuth,
                      params,
                      redirectUri,
                      codeVerifier,
                    );

                  const result =
                    await oauth.processAuthorizationCodeResponse(
                      as,
                      client,
                      response,
                    );

                  // Calculate expiration time
                  const expiresAt = result.expires_in
                    ? new Date(
                        Date.now() + result.expires_in * 1000,
                      ).toISOString()
                    : null;

                  // Parse scopes from space-separated string into array
                  const scopes = result.scope
                    ? result.scope.split(" ").filter(Boolean)
                    : null;

                  // Write tokens and client credentials to a single file
                  // for the Rust MCP server. Including client credentials
                  // enables the server to refresh tokens without requiring
                  // separate configuration.
                  saveTokens({
                    access_token: result.access_token,
                    refresh_token: result.refresh_token ?? "",
                    expires_at: expiresAt,
                    token_type: result.token_type || "bearer",
                    scopes,
                    client_id: clientId,
                    client_secret: clientSecret,
                  });

                  return {
                    type: "success" as const,
                    access: result.access_token,
                    refresh: result.refresh_token ?? "",
                    expires: result.expires_in ?? 3600,
                  };
                } catch (err) {
                  const msg = err instanceof Error ? err.message : String(err);
                  console.error(`Authorization failed: ${msg}`);
                  return { type: "failed" as const };
                } finally {
                  server.stop();
                }
              },
            };
          },
        },
      ],
    },
  };
};

export default OnshapeAuthPlugin;
