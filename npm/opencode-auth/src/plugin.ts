import type { Plugin } from "@opencode-ai/plugin";
import { mkdirSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { homedir } from "os";
import * as oauth from "oauth4webapi";

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
            `<html><body><h1>Authorization Failed</h1><p>${error}</p><p>You can close this window.</p></body></html>`,
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

            // 2. Generate PKCE verifier/challenge and CSRF state.
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
                  const exchangeResp = await fetch(
                    `${proxyUrl}/token/exchange`,
                    {
                      method: "POST",
                      headers: { "Content-Type": "application/json" },
                      body: JSON.stringify({
                        code,
                        redirect_uri: redirectUri,
                        code_verifier: codeVerifier,
                      }),
                    },
                  );

                  if (!exchangeResp.ok) {
                    const errBody = await exchangeResp.text();
                    if (exchangeResp.status === 403) {
                      // Parse the source_ip from the proxy's 403 response
                      // to give the user actionable debugging information.
                      let sourceIpHint = "";
                      try {
                        const parsed = JSON.parse(errBody);
                        if (parsed.source_ip) {
                          sourceIpHint = ` Your public IP is ${parsed.source_ip}.`;
                        }
                      } catch {
                        // Response wasn't JSON — use the raw body in the error.
                      }
                      throw new Error(
                        `Proxy rejected request (403 Forbidden).${sourceIpHint} Ensure the proxy's ALLOWED_SOURCES includes your IP address.`,
                      );
                    }
                    throw new Error(
                      `Proxy token exchange failed: ${exchangeResp.status} ${errBody}`,
                    );
                  }

                  const result = (await exchangeResp.json()) as {
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
                  console.error("Authorization failed:", err);
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
                  console.error("Authorization failed:", err);
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
