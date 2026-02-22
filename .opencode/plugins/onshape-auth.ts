import type { Plugin } from "@opencode-ai/plugin";
import { mkdirSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { homedir } from "os";

// Onshape OAuth constants (must match crates/onshape-client-core/src/oauth.rs)
const ONSHAPE_AUTH_URL = "https://oauth.onshape.com/oauth/authorize";
const ONSHAPE_TOKEN_URL = "https://oauth.onshape.com/oauth/token";
const OAUTH_SCOPES = "OAuth2Read OAuth2Write";

/**
 * Returns the default token file path for the current platform.
 * Mirrors `default_token_file_path()` in onshape-client-core.
 */
function tokenFilePath(): string {
  const platform = process.platform;
  const home = homedir();

  if (platform === "darwin") {
    return join(home, "Library", "Application Support", "onshape-mcp", "tokens.json");
  } else if (platform === "win32") {
    return join(process.env.LOCALAPPDATA || join(home, "AppData", "Local"), "onshape-mcp", "tokens.json");
  } else {
    // Linux and other Unix
    return join(process.env.XDG_DATA_HOME || join(home, ".local", "share"), "onshape-mcp", "tokens.json");
  }
}

/**
 * Save tokens to the token file with secure permissions.
 */
function saveTokens(tokens: {
  access_token: string;
  refresh_token: string;
  expires_at: string | null;
  token_type: string;
}): void {
  const path = tokenFilePath();
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, JSON.stringify(tokens, null, 2), { mode: 0o600 });
}

export const OnshapeAuthPlugin: Plugin = async (_ctx) => {
  return {
    auth: {
      provider: "onshape",
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

            // Start a local HTTP server for the OAuth callback
            const server = Bun.serve({
              port: 0, // Auto-assign a free port
              async fetch(req) {
                const url = new URL(req.url);
                if (url.pathname === "/callback") {
                  const code = url.searchParams.get("code");
                  const error = url.searchParams.get("error");

                  if (error) {
                    return new Response(
                      `<html><body><h1>Authorization Failed</h1><p>${error}</p><p>You can close this window.</p></body></html>`,
                      { headers: { "Content-Type": "text/html" } }
                    );
                  }

                  if (code) {
                    // Store the code for the callback to pick up
                    (server as any)._authCode = code;
                    return new Response(
                      `<html><body><h1>Authorization Successful</h1><p>You can close this window and return to your terminal.</p></body></html>`,
                      { headers: { "Content-Type": "text/html" } }
                    );
                  }

                  return new Response("Missing code parameter", { status: 400 });
                }
                return new Response("Not found", { status: 404 });
              },
            });

            const port = server.port;
            const redirectUri = `http://localhost:${port}/callback`;

            // Build the authorization URL
            const authUrl = new URL(ONSHAPE_AUTH_URL);
            authUrl.searchParams.set("client_id", clientId);
            authUrl.searchParams.set("response_type", "code");
            authUrl.searchParams.set("redirect_uri", redirectUri);
            authUrl.searchParams.set("scope", OAUTH_SCOPES);

            return {
              url: authUrl.toString(),
              instructions: "Open the URL above to authorize with Onshape. After granting access, you will be redirected back automatically.",
              method: "auto" as const,
              async callback() {
                try {
                  // Poll for the authorization code
                  const deadline = Date.now() + 120_000; // 2 minute timeout
                  while (Date.now() < deadline) {
                    const code = (server as any)._authCode;
                    if (code) {
                      // Exchange the code for tokens
                      const tokenResponse = await fetch(ONSHAPE_TOKEN_URL, {
                        method: "POST",
                        headers: {
                          "Content-Type": "application/x-www-form-urlencoded",
                        },
                        body: new URLSearchParams({
                          grant_type: "authorization_code",
                          code,
                          redirect_uri: redirectUri,
                          client_id: clientId,
                          client_secret: clientSecret,
                        }).toString(),
                      });

                      if (!tokenResponse.ok) {
                        const errorText = await tokenResponse.text();
                        console.error("Token exchange failed:", errorText);
                        return { type: "failed" as const };
                      }

                      const tokenData = (await tokenResponse.json()) as {
                        access_token: string;
                        refresh_token: string;
                        expires_in?: number;
                        token_type: string;
                      };

                      // Calculate expiration time
                      const expiresAt = tokenData.expires_in
                        ? new Date(Date.now() + tokenData.expires_in * 1000).toISOString()
                        : null;

                      // Write tokens to the token file for the Rust MCP server
                      saveTokens({
                        access_token: tokenData.access_token,
                        refresh_token: tokenData.refresh_token,
                        expires_at: expiresAt,
                        token_type: tokenData.token_type || "Bearer",
                      });

                      return {
                        type: "success" as const,
                        access: tokenData.access_token,
                        refresh: tokenData.refresh_token,
                        expires: tokenData.expires_in || 3600,
                      };
                    }
                    await Bun.sleep(500);
                  }

                  // Timeout
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
