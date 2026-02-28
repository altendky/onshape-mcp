/**
 * DNS-aware proxy request helpers.
 *
 * ## Why this exists
 *
 * The OAuth proxy restricts access by source IP (`ALLOWED_SOURCES`).
 * On dual-stack networks the OS typically prefers IPv6, so `fetch()`
 * may connect via IPv6 while `ALLOWED_SOURCES` only lists IPv4
 * addresses (or vice versa), resulting in a 403.
 *
 * The Rust MCP server solves this with reqwest's
 * `local_address(Ipv4Addr::UNSPECIFIED)` to force IPv4 on retry.
 * In Bun we have no working equivalent — both of the standard
 * approaches are silently ignored (as of Bun 1.3):
 *
 *   - `undici` Agent with `connect: { localAddress: "0.0.0.0" }`
 *   - `node:https` `request()` with `family: 4`
 *
 * ## What we do instead
 *
 * We resolve the hostname ourselves via `dns.resolve4` / `dns.resolve6`
 * and connect to each resolved IP directly with `node:https`, setting
 * `servername` for TLS SNI and `Host` for HTTP routing.
 *
 * The caller tries IPv6 first (matching OS default preference), then
 * falls back to IPv4 if the proxy returns 403 or all IPv6 addresses
 * fail to connect.  Within a family, only connection errors advance to
 * the next address — any HTTP response (even 403) is a definitive
 * answer from the proxy.
 */

import { resolve4, resolve6 } from "node:dns/promises";
import { request as httpsRequest } from "node:https";

// ============================================================================
// Proxy error response parsing
// ============================================================================

/** Structured representation of a proxy error response. */
export interface ProxyErrorResponse {
  error: string;
  source_ip?: string;
}

/**
 * Parse a response body as a proxy JSON error.
 *
 * Returns the parsed object if the body is valid JSON with at least an
 * `error` field, or `null` if the body cannot be parsed.
 */
export function parseProxyError(body: string): ProxyErrorResponse | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return null;
  }

  if (
    typeof parsed !== "object" ||
    parsed === null ||
    Array.isArray(parsed)
  ) {
    return null;
  }

  const obj = parsed as Record<string, unknown>;
  if (typeof obj.error !== "string") {
    return null;
  }

  const result: ProxyErrorResponse = { error: obj.error };
  if (typeof obj.source_ip === "string" && obj.source_ip !== "") {
    result.source_ip = obj.source_ip;
  }
  return result;
}

/**
 * Check whether a source IP string looks like IPv6.
 *
 * IPv6 addresses always contain a colon; IPv4 never does.
 */
export function isIpv6(ip: string): boolean {
  return ip.includes(":");
}

/**
 * Determine whether an IPv4 retry is appropriate based on a proxy 403
 * response body.
 *
 * Returns `true` when the response contains an IPv6 `source_ip`,
 * meaning the proxy rejected us because it saw an IPv6 address that
 * isn't in ALLOWED_SOURCES.
 */
export function shouldRetryIpv4(responseBody: string): boolean {
  const parsed = parseProxyError(responseBody);
  if (!parsed?.source_ip) return false;
  return isIpv6(parsed.source_ip);
}

// ============================================================================
// DNS resolution
// ============================================================================

/**
 * Resolve a hostname to all IPv4 (A record) addresses.
 *
 * Returns an empty array if DNS resolution fails or yields no results.
 */
export async function resolveIpv4(hostname: string): Promise<string[]> {
  try {
    return await resolve4(hostname);
  } catch {
    return [];
  }
}

/**
 * Resolve a hostname to all IPv6 (AAAA record) addresses.
 *
 * Returns an empty array if DNS resolution fails or yields no results.
 */
export async function resolveIpv6(hostname: string): Promise<string[]> {
  try {
    return await resolve6(hostname);
  } catch {
    return [];
  }
}

// ============================================================================
// HTTPS POST to a specific IP
// ============================================================================

/** Result of an HTTPS POST request. */
export interface HttpResult {
  status: number;
  body: string;
}

/**
 * Make an HTTPS POST request with JSON body to a specific IP address.
 *
 * The request is sent directly to `ipAddress` with `servername` set
 * for TLS SNI and `Host` set for HTTP routing, so Cloudflare (or any
 * SNI-based host) routes the request correctly despite connecting to
 * an IP rather than a hostname.
 *
 * Works for both IPv4 and IPv6 addresses.
 *
 * Returns the response status and body text.  Throws on network errors.
 */
export function httpsPostJsonToIp(
  ipAddress: string,
  url: string,
  jsonBody: unknown,
): Promise<HttpResult> {
  const parsed = new URL(url);
  const body = JSON.stringify(jsonBody);
  return httpsPostToIp(ipAddress, parsed, body);
}

/**
 * Low-level POST to a specific IP with pre-parsed URL and serialised body.
 *
 * Separated from `httpsPostJsonToIp` so that callers which iterate
 * over multiple addresses (see `tryAddresses`) can parse once and
 * avoid re-throwing parse errors on every address.
 */
function httpsPostToIp(
  ipAddress: string,
  parsed: URL,
  body: string,
): Promise<HttpResult> {
  const hostname = parsed.hostname;
  const port = parsed.port || 443;

  return new Promise((resolve, reject) => {
    const req = httpsRequest(
      {
        hostname: ipAddress,
        port,
        path: parsed.pathname + parsed.search,
        method: "POST",
        servername: hostname, // TLS SNI must match the certificate
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
          Host: hostname,
        },
      },
      (res) => {
        let data = "";
        res.on("data", (chunk: string) => (data += chunk));
        res.on("end", () => {
          resolve({ status: res.statusCode ?? 0, body: data });
        });
      },
    );

    req.on("error", reject);
    req.write(body);
    req.end();
  });
}

// ============================================================================
// Try a list of addresses, stopping on the first HTTP response
// ============================================================================

/**
 * Try POSTing to each address in order.  Stop on the first HTTP
 * response (even an error like 403).  Only connection-level failures
 * advance to the next address.
 *
 * Returns the result from the first address that responded, or `null`
 * if every address failed at the connection level.
 */
export async function tryAddresses(
  addresses: string[],
  url: string,
  jsonBody: unknown,
): Promise<HttpResult | null> {
  // Parse URL and serialise body once before the loop.  Errors here
  // (malformed URL, circular JSON) propagate immediately to the caller
  // instead of being silently swallowed by the per-address catch below.
  const parsed = new URL(url);
  const body = JSON.stringify(jsonBody);

  for (const addr of addresses) {
    try {
      return await httpsPostToIp(addr, parsed, body);
    } catch {
      // Connection error — try the next address.
      continue;
    }
  }
  return null;
}
