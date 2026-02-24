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
 * Save tokens and client credentials to the token file with secure permissions.
 * Client credentials are included so the MCP server can refresh tokens
 * without requiring separate configuration.
 */
function saveTokens(tokens: {
  access_token: string;
  refresh_token: string;
  expires_at: string | null;
  token_type: string;
  scopes: string[] | null;
  client_id: string;
  client_secret: string;
}): void {
  const path = tokenFilePath();
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(tokens, null, 2), { mode: 0o600 });
}

export const OnshapeAuthPlugin: Plugin = async (_ctx) => {
  return {
    auth: {
      provider: "OnShape (onshape-mcp)",
      methods: [
        {
          type: "oauth" as const,
          label: "Onshape OAuth",
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
            const server = Bun.serve({
              port: 18338, // Fixed port to match Onshape OAuth app redirect URL registration
              async fetch(req) {
                const url = new URL(req.url);
                if (url.pathname === "/callback") {
                  const error = url.searchParams.get("error");

                  if (error) {
                    // Quick check for user-facing HTML; authoritative
                    // validation happens via oauth4webapi in the polling loop
                    (server as any)._callbackUrl = url;
                    return new Response(
                      `<html><body><h1>Authorization Failed</h1><p>${error}</p><p>You can close this window.</p></body></html>`,
                      { headers: { "Content-Type": "text/html" } },
                    );
                  }

                  // Store the full callback URL for the polling loop to validate
                  (server as any)._callbackUrl = url;
                  return new Response(
                    `<html><body><h1>Authorization Successful</h1><p>You can close this window and return to your terminal.</p></body></html>`,
                    { headers: { "Content-Type": "text/html" } },
                  );
                }
                return new Response("Not found", { status: 404 });
              },
            });

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
                  // Poll for the callback URL from the local server
                  const deadline = Date.now() + 120_000; // 2 minute timeout
                  while (Date.now() < deadline) {
                    const callbackUrl: URL | undefined =
                      (server as any)._callbackUrl;
                    if (callbackUrl) {
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
                    }
                    await Bun.sleep(500);
                  }

                  // Timeout
                  return { type: "failed" as const };
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
