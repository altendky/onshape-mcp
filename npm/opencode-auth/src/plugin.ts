import type { Plugin } from "@opencode-ai/plugin";
import {
  closeSync,
  chmodSync,
  fsyncSync,
  mkdirSync,
  openSync,
  renameSync,
  rmdirSync,
  unlinkSync,
  writeFileSync,
} from "fs";
import { randomUUID } from "crypto";
import { basename, dirname, join, posix, win32 } from "path";
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
const TOKEN_REPLACE_RETRY_MS = 25;
const TOKEN_REPLACE_TIMEOUT_MS = 1_000;

/**
 * Returns the data directory for onshape-mcp on the current platform.
 * Mirrors `default_token_file_path()` logic in onshape-mcp-io.
 */
export function resolveDataDir(
  platform: NodeJS.Platform,
  home: string,
  environment: NodeJS.ProcessEnv,
): string {
  const paths = platform === "win32" ? win32 : posix;
  const xdgDataHome = environment.XDG_DATA_HOME;
  const xdgRoot = xdgDataHome ? paths.parse(xdgDataHome).root : "";
  const isAbsoluteXdgDataHome =
    xdgDataHome &&
    paths.isAbsolute(xdgDataHome) &&
    (platform !== "win32" || (xdgRoot !== "\\" && xdgRoot !== "/"));
  if (isAbsoluteXdgDataHome) {
    return paths.join(xdgDataHome, "onshape-mcp");
  }

  if (platform === "darwin") {
    return paths.join(home, "Library", "Application Support", "onshape-mcp");
  } else if (platform === "win32") {
    return paths.join(
      environment.LOCALAPPDATA || paths.join(home, "AppData", "Local"),
      "onshape-mcp",
    );
  } else {
    // Linux and other Unix
    return paths.join(home, ".local", "share", "onshape-mcp");
  }
}

function tokenFilePath(): string {
  return join(resolveDataDir(process.platform, homedir(), process.env), "tokens.json");
}

/**
 * Token file shape.  Either `client_secret` (direct mode) or `proxy_url`
 * (proxy mode) is set, never both.
 */
export interface TokenFile {
  access_token: string;
  refresh_token: string;
  expires_at: string | null;
  token_type: string;
  scopes: string[] | null;
  client_id: string;
  client_secret?: string;
  proxy_url?: string;
}

interface ProxyTokenResponse {
  access_token: string;
  refresh_token: string;
  token_type?: "bearer";
  expires_in?: number;
  scope?: string;
}

export function absoluteOAuthExpiry(
  expiresIn: number | undefined,
  now = Date.now(),
): number {
  return now + (expiresIn ?? 3600) * 1000;
}

export function parseProxyTokenResponse(body: string): ProxyTokenResponse {
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    throw new Error("Proxy token exchange returned invalid JSON");
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error(
      "Proxy token exchange returned invalid token payload: expected an object",
    );
  }
  const payload = parsed as Record<string, unknown>;

  if (
    typeof payload.access_token !== "string" ||
    payload.access_token.trim() === ""
  ) {
    throw new Error(
      "Proxy token exchange returned invalid token payload: missing or empty access_token",
    );
  }
  if (
    typeof payload.refresh_token !== "string" ||
    payload.refresh_token.trim() === ""
  ) {
    throw new Error(
      "Proxy token exchange returned invalid token payload: missing or empty refresh_token",
    );
  }

  let tokenType: "bearer" | undefined;
  if (payload.token_type !== undefined) {
    if (
      typeof payload.token_type !== "string" ||
      payload.token_type.trim() === "" ||
      payload.token_type.toLowerCase() !== "bearer"
    ) {
      throw new Error(
        "Proxy token exchange returned invalid token payload: token_type must be bearer",
      );
    }
    tokenType = "bearer";
  }

  let expiresIn: number | undefined;
  if (payload.expires_in !== undefined) {
    if (
      typeof payload.expires_in !== "number" ||
      !Number.isFinite(payload.expires_in) ||
      payload.expires_in < 0
    ) {
      throw new Error(
        "Proxy token exchange returned invalid token payload: expires_in must be a finite nonnegative number",
      );
    }
    expiresIn = payload.expires_in;
  }

  let scope: string | undefined;
  if (payload.scope !== undefined) {
    if (typeof payload.scope !== "string" || payload.scope.trim() === "") {
      throw new Error(
        "Proxy token exchange returned invalid token payload: scope must be a nonempty string",
      );
    }
    scope = payload.scope;
  }

  return {
    access_token: payload.access_token,
    refresh_token: payload.refresh_token,
    ...(tokenType === undefined ? {} : { token_type: tokenType }),
    ...(expiresIn === undefined ? {} : { expires_in: expiresIn }),
    ...(scope === undefined ? {} : { scope }),
  };
}

const TOKEN_LOCK_RETRY_MS = 25;
const TOKEN_LOCK_TIMEOUT_MS = 75_000;
const TOKEN_EXCHANGE_TIMEOUT_MS = 30_000;
const PROXY_EXCHANGE_TRANSACTION_TIMEOUT_MS = 60_000;

export function tokenLockPath(path: string): string {
  return `${path}.lock`;
}

async function settle<T>(
  operation: () => Promise<T>,
): Promise<PromiseSettledResult<T>> {
  try {
    return { status: "fulfilled", value: await operation() };
  } catch (reason) {
    return { status: "rejected", reason };
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function completeWithCleanup<T>(
  operationResult: PromiseSettledResult<T>,
  cleanupErrors: unknown[],
): T {
  if (operationResult.status === "rejected") {
    if (cleanupErrors.length > 0) {
      throw new AggregateError(
        [operationResult.reason, ...cleanupErrors],
        `Operation failed: ${errorMessage(operationResult.reason)}; cleanup failed: ${cleanupErrors.map(errorMessage).join("; ")}`,
        { cause: operationResult.reason },
      );
    }
    throw operationResult.reason;
  }

  if (cleanupErrors.length === 1) throw cleanupErrors[0];
  if (cleanupErrors.length > 1) {
    throw new AggregateError(
      cleanupErrors,
      `Cleanup failed: ${cleanupErrors.map(errorMessage).join("; ")}`,
    );
  }
  return operationResult.value;
}

export function validateProxyUrl(value: string): string {
  const proxyInput = value.trim();
  if (!proxyInput) {
    throw new Error("Self-hosted OAuth Proxy URL is required");
  }
  const parsedProxyUrl = new URL(proxyInput);
  const hostname = parsedProxyUrl.hostname.toLowerCase();
  const isLoopback =
    hostname === "localhost" ||
    hostname === "[::1]" ||
    /^127(?:\.\d{1,3}){3}$/.test(hostname);
  if (
    parsedProxyUrl.protocol !== "https:" &&
    !(parsedProxyUrl.protocol === "http:" && isLoopback)
  ) {
    throw new Error(
      "OAuth Proxy URL must use https:// (http:// is only allowed for loopback hosts)",
    );
  }
  return parsedProxyUrl.origin + parsedProxyUrl.pathname.replace(/\/$/, "");
}

interface TokenFileLock {
  path: string;
  lockPath: string;
  ownerPath: string;
  cleanupPath: string;
}

export async function publishTokenFile(
  publication: () => void,
  platform: NodeJS.Platform = process.platform,
  timeoutMs = TOKEN_REPLACE_TIMEOUT_MS,
): Promise<void> {
  const deadline = Bun.nanoseconds() + timeoutMs * 1_000_000;
  while (true) {
    try {
      publication();
      return;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (
        platform !== "win32" ||
        (code !== "EACCES" && code !== "EPERM" && code !== "EBUSY") ||
        Bun.nanoseconds() >= deadline
      ) {
        throw error;
      }
      await Bun.sleep(TOKEN_REPLACE_RETRY_MS);
    }
  }
}

export async function withTokenFileLock<T>(
  operation: (lock: TokenFileLock) => Promise<T>,
  path = tokenFilePath(),
  lockTimeoutMs = TOKEN_LOCK_TIMEOUT_MS,
): Promise<T> {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const lockPath = tokenLockPath(path);
  const deadline = Bun.nanoseconds() + lockTimeoutMs * 1_000_000;
  let lock: TokenFileLock | undefined;

  while (true) {
    try {
      mkdirSync(lockPath, { mode: 0o700 });
      const ownerPath = join(lockPath, `owner-${process.pid}-${randomUUID()}`);
      try {
        mkdirSync(ownerPath, { mode: 0o700 });
      } catch (error) {
        const cleanupErrors: unknown[] = [];
        try {
          rmdirSync(lockPath);
        } catch (cleanupError) {
          cleanupErrors.push(cleanupError);
        }
        return completeWithCleanup<T>(
          { status: "rejected", reason: error },
          cleanupErrors,
        );
      }
      lock = {
        path,
        lockPath,
        ownerPath,
        cleanupPath: `${lockPath}.cleanup-${process.pid}-${randomUUID()}`,
      };
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;

      if (Bun.nanoseconds() >= deadline) {
        throw new Error(
          `Timed out waiting for token file lock directory ${lockPath}. Stop all onshape-mcp/OpenCode writers, then manually remove the lock directory if no writer is running.`,
        );
      }
      await Bun.sleep(TOKEN_LOCK_RETRY_MS);
    }
  }
  if (!lock) throw new Error("Token lock acquisition ended without a lock");

  const operationResult = await settle(() => operation(lock));
  const cleanupErrors: unknown[] = [];
  try {
    renameSync(lock.lockPath, lock.cleanupPath);
    try {
      rmdirSync(join(lock.cleanupPath, basename(lock.ownerPath)));
      rmdirSync(lock.cleanupPath);
    } catch (error) {
      cleanupErrors.push(error);
      try {
        renameSync(lock.cleanupPath, lock.lockPath);
      } catch (restoreError) {
        cleanupErrors.push(restoreError);
      }
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      cleanupErrors.push(error);
    }
  }
  return completeWithCleanup(operationResult, cleanupErrors);
}

async function saveTokensLocked(
  tokens: TokenFile,
  path: string,
  lock: TokenFileLock,
): Promise<void> {
  if (lock.path !== path) {
    throw new Error(`Token lock does not protect ${path}`);
  }
  let temporaryPath: string | undefined;
  let descriptor: number | undefined;
  const operationResult = await settle(async () => {
    const candidatePath = join(
      dirname(path),
      `.${basename(path)}.tmp-${process.pid}-${randomUUID()}`,
    );
    descriptor = openSync(candidatePath, "wx", 0o600);
    temporaryPath = candidatePath;
    writeFileSync(descriptor, JSON.stringify(tokens, null, 2));
    fsyncSync(descriptor);
    chmodSync(candidatePath, 0o600);
    closeSync(descriptor);
    descriptor = undefined;
    await publishTokenFile(() => renameSync(candidatePath, path));
    temporaryPath = undefined;
    chmodSync(path, 0o600);
  });

  const cleanupErrors: unknown[] = [];
  if (descriptor !== undefined) {
    try {
      closeSync(descriptor);
    } catch (error) {
      cleanupErrors.push(error);
    }
  }
  if (temporaryPath !== undefined) {
    try {
      unlinkSync(temporaryPath);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        cleanupErrors.push(error);
      }
    }
  }
  completeWithCleanup(operationResult, cleanupErrors);
}

/** Save tokens atomically while holding the shared credential lock. */
export async function saveTokens(
  tokens: TokenFile,
  path = tokenFilePath(),
  lockTimeoutMs = TOKEN_LOCK_TIMEOUT_MS,
): Promise<void> {
  await withTokenFileLock(
    (lock) => saveTokensLocked(tokens, path, lock),
    path,
    lockTimeoutMs,
  );
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
        // Self-hosted proxy mode: client secret held by the proxy server.
        // ================================================================
        {
          type: "oauth" as const,
          label: "Onshape OAuth (self-hosted proxy)",
          prompts: [
            {
              type: "text" as const,
              key: "proxy_url",
              message: "OAuth Proxy URL",
              placeholder: "https://oauth-proxy.example.com",
            },
          ],
          async authorize(inputs) {
            const proxyUrl = validateProxyUrl(inputs?.proxy_url ?? "");

            // 1. Resolve proxy hostname to IPv6 and IPv4 addresses
            //    once, then reuse for both /config and /token/exchange.
            const proxyHostname = new URL(proxyUrl).hostname;
            const [ipv6Addrs, ipv4Addrs] = await Promise.all([
              resolveIpv6(proxyHostname),
              resolveIpv4(proxyHostname),
            ]);

            // 2. Fetch client_id from the proxy's /config endpoint.
            //    This endpoint is IP-restricted, so a 403 here means
            //    our IP is not authorized — fail early with a clear
            //    message instead of sending the user to the browser.
            const configUrl = `${proxyUrl}/config`;
            const triedSourceIps: string[] = [];
            const collect403Ip = (body: string) => {
              const err = parseProxyError(body);
              if (err?.source_ip) triedSourceIps.push(err.source_ip);
            };

            const ipv6Config = await tryAddresses(
              ipv6Addrs,
              configUrl,
              null,
              "GET",
            );

            let configStatus: number | undefined;
            let configBody: string | undefined;

            if (ipv6Config && ipv6Config.status !== 403) {
              configStatus = ipv6Config.status;
              configBody = ipv6Config.body;
            } else {
              if (ipv6Config?.status === 403) {
                collect403Ip(ipv6Config.body);
              }

              const ipv4Config = await tryAddresses(
                ipv4Addrs,
                configUrl,
                null,
                "GET",
              );

              if (ipv4Config) {
                configStatus = ipv4Config.status;
                configBody = ipv4Config.body;
                if (ipv4Config.status === 403) {
                  collect403Ip(ipv4Config.body);
                }
              } else if (ipv6Config) {
                configStatus = ipv6Config.status;
                configBody = ipv6Config.body;
              }
            }

            // Handle /config failures.
            if (configStatus === undefined) {
              return {
                url: "-",
                instructions: `Failed to connect to proxy at ${proxyHostname}.`,
                method: "auto" as const,
                async callback() {
                  return { type: "failed" as const };
                },
              };
            }

            if (configStatus === 403) {
              const ipNote =
                triedSourceIps.length > 0
                  ? ` (source IP: ${triedSourceIps.join(", ")})`
                  : "";
              return {
                url: "-",
                instructions: `Your IP address${ipNote} is not authorized to use this proxy.`,
                method: "auto" as const,
                async callback() {
                  return { type: "failed" as const };
                },
              };
            }

            if (configStatus !== 200) {
              throw new Error(
                `Failed to fetch proxy config: ${configStatus} ${configBody}`,
              );
            }

            let proxyConfig: { client_id: string };
            try {
              proxyConfig = JSON.parse(configBody!) as {
                client_id: string;
              };
            } catch {
              throw new Error(
                `Proxy /config returned invalid JSON: ${configBody}`,
              );
            }
            const clientId = proxyConfig.client_id;

            if (typeof clientId !== "string" || clientId === "") {
              throw new Error(
                "Proxy /config response missing or invalid 'client_id' field",
              );
            }

            // 3. Generate PKCE verifier/challenge and CSRF state.
            const state = oauth.generateRandomState();
            const codeVerifier = oauth.generateRandomCodeVerifier();
            const codeChallenge =
              await oauth.calculatePKCECodeChallenge(codeVerifier);

            // 4. Start localhost callback server.
            const server = startCallbackServer();
            const port = server.port;
            const redirectUri = `http://localhost:${port}/callback`;

            // 5. Build the authorization URL.
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

                  const tokenPath = tokenFilePath();
                  return await withTokenFileLock(async (tokenLock) => {
                    // 6. Exchange via the proxy (not directly with Onshape).
                    //    Uses the same DNS-aware dual-stack retry as the
                    //    /config fetch above.  See ipv4-retry.ts for details.
                    const exchangeUrl = `${proxyUrl}/token/exchange`;
                    const exchangeBody = {
                      code,
                      redirect_uri: redirectUri,
                      code_verifier: codeVerifier,
                    };

                    const exchangeSourceIps: string[] = [];
                    const exchangeDeadline =
                      Date.now() + PROXY_EXCHANGE_TRANSACTION_TIMEOUT_MS;
                    let respStatus: number | undefined;
                    let respBody: string | undefined;

                    const collectExchange403Ip = (body: string) => {
                      const err = parseProxyError(body);
                      if (err?.source_ip) exchangeSourceIps.push(err.source_ip);
                    };

                    // Try IPv6 addresses first (matches OS default preference).
                    const ipv6Result = await tryAddresses(
                      ipv6Addrs,
                      exchangeUrl,
                      exchangeBody,
                      "POST",
                      exchangeDeadline,
                    );

                    if (ipv6Result && ipv6Result.status !== 403) {
                      // Got a non-403 response over IPv6 — use it.
                      respStatus = ipv6Result.status;
                      respBody = ipv6Result.body;
                    } else {
                      // Either all IPv6 addrs had connection errors, or
                      // we got 403.  Record the 403 IP and try IPv4.
                      if (ipv6Result?.status === 403) {
                        collectExchange403Ip(ipv6Result.body);
                      }

                      const ipv4Result = await tryAddresses(
                        ipv4Addrs,
                        exchangeUrl,
                        exchangeBody,
                        "POST",
                        exchangeDeadline,
                      );

                      if (ipv4Result) {
                        respStatus = ipv4Result.status;
                        respBody = ipv4Result.body;
                        if (ipv4Result.status === 403) {
                          collectExchange403Ip(ipv4Result.body);
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
                        const ipNote =
                          exchangeSourceIps.length > 0
                            ? ` (source IP: ${exchangeSourceIps.join(", ")})`
                            : "";
                        throw new Error(
                          `Your IP address${ipNote} is not authorized to use this proxy.`,
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

                    if (respBody === undefined) {
                      throw new Error(
                        "Proxy token exchange returned no response body",
                      );
                    }
                    const result = parseProxyTokenResponse(respBody);

                    const expires = absoluteOAuthExpiry(result.expires_in);
                    const expiresAt =
                      result.expires_in !== undefined
                        ? new Date(expires).toISOString()
                        : null;

                    const scopes = result.scope !== undefined
                      ? result.scope.split(" ").filter(Boolean)
                      : null;

                    // 7. Save tokens with proxy_url (not client_secret).
                    await saveTokensLocked(
                      {
                        access_token: result.access_token,
                        refresh_token: result.refresh_token,
                        expires_at: expiresAt,
                        token_type: result.token_type ?? "bearer",
                        scopes,
                        client_id: clientId,
                        proxy_url: proxyUrl,
                      },
                      tokenPath,
                      tokenLock,
                    );

                    return {
                      type: "success" as const,
                      access: result.access_token,
                      refresh: result.refresh_token,
                      expires,
                    };
                  }, tokenPath);
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

            if (!clientId?.trim() || !clientSecret?.trim()) {
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
              instructions:
                "Open the URL above to authorize with Onshape. After granting access, you will be redirected back automatically.",
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

                  const tokenPath = tokenFilePath();
                  return await withTokenFileLock(async (tokenLock) => {
                    // Exchange the authorization code for tokens
                    const response = await oauth.authorizationCodeGrantRequest(
                      as,
                      client,
                      clientAuth,
                      params,
                      redirectUri,
                      codeVerifier,
                      {
                        signal: AbortSignal.timeout(TOKEN_EXCHANGE_TIMEOUT_MS),
                      },
                    );

                    const result = await oauth.processAuthorizationCodeResponse(
                      as,
                      client,
                      response,
                    );
                    if (!result.refresh_token?.trim()) {
                      throw new Error(
                        "Onshape token exchange returned missing or empty refresh_token",
                      );
                    }

                    // OpenCode expects an absolute Unix timestamp in milliseconds.
                    const expires = absoluteOAuthExpiry(result.expires_in);
                    const expiresAt =
                      result.expires_in !== undefined
                        ? new Date(expires).toISOString()
                        : null;

                    // Parse scopes from space-separated string into array
                    const scopes = result.scope
                      ? result.scope.split(" ").filter(Boolean)
                      : null;

                    // Write tokens and client credentials to a single file
                    // for the Rust MCP server. Including client credentials
                    // enables the server to refresh tokens without requiring
                    // separate configuration.
                    await saveTokensLocked(
                      {
                        access_token: result.access_token,
                        refresh_token: result.refresh_token,
                        expires_at: expiresAt,
                        token_type: result.token_type || "bearer",
                        scopes,
                        client_id: clientId,
                        client_secret: clientSecret,
                      },
                      tokenPath,
                      tokenLock,
                    );

                    return {
                      type: "success" as const,
                      access: result.access_token,
                      refresh: result.refresh_token,
                      expires,
                    };
                  }, tokenPath);
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
      ].reverse(),
    },
  };
};

export default OnshapeAuthPlugin;
