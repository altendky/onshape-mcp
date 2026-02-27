/**
 * Pure request handler — no I/O, no fetch, no DNS resolution.
 *
 * All functions take plain data and return Effect descriptors.
 * The I/O layer (index.ts) is responsible for executing effects.
 */

import type {
  AllowedSources,
  Effect,
  Env,
  ExchangeRequestBody,
  RefreshRequestBody,
  RequestContext,
} from "./types.js";
import { ONSHAPE_TOKEN_URL } from "./types.js";

// ============================================================================
// IP / Allowed Sources
// ============================================================================

/**
 * Parse the ALLOWED_SOURCES string into IPs and hostnames.
 *
 * Each comma-separated entry is classified: if it looks like an IPv4 or IPv6
 * address it goes into `ips`, otherwise into `hostnames` (for DNS resolution).
 */
export function parseAllowedSources(raw: string): AllowedSources {
  const ips: string[] = [];
  const hostnames: string[] = [];

  for (const entry of raw.split(",")) {
    const trimmed = entry.trim();
    if (trimmed === "") continue;

    if (isIpAddress(trimmed)) {
      ips.push(trimmed);
    } else {
      hostnames.push(trimmed);
    }
  }

  return { ips, hostnames };
}

/**
 * Check whether a string looks like an IP address (v4 or v6).
 *
 * Simple heuristic: IPv4 contains only digits and dots with at least one dot,
 * IPv6 contains a colon.  This is sufficient for classifying ALLOWED_SOURCES
 * entries — actual IP validation happens at comparison time.
 */
function isIpAddress(value: string): boolean {
  // IPv6: contains colon
  if (value.includes(":")) return true;
  // IPv4: digits and dots, at least one dot
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(value)) return true;
  return false;
}

/** Check whether a source IP is in the resolved allowed-IP list. */
export function isIpAllowed(sourceIp: string, allowedIps: string[]): boolean {
  return allowedIps.includes(sourceIp);
}

// ============================================================================
// Route Handlers
// ============================================================================

function handleHealth(): Effect {
  return { type: "json-response", status: 200, body: { status: "ok" } };
}

function handleConfig(env: Env): Effect {
  return {
    type: "json-response",
    status: 200,
    body: { client_id: env.ONSHAPE_CLIENT_ID },
  };
}

function handleExchange(body: unknown, env: Env): Effect {
  if (!isObject(body)) {
    return jsonError(400, "request body must be a JSON object");
  }

  const { code, redirect_uri, code_verifier } = body as Record<string, unknown>;

  if (typeof code !== "string" || code === "") {
    return jsonError(400, "missing or invalid 'code' field");
  }
  if (typeof redirect_uri !== "string" || redirect_uri === "") {
    return jsonError(400, "missing or invalid 'redirect_uri' field");
  }
  if (code_verifier !== undefined && typeof code_verifier !== "string") {
    return jsonError(400, "'code_verifier' must be a string if provided");
  }

  const formBody = new URLSearchParams();
  formBody.set("grant_type", "authorization_code");
  formBody.set("code", code);
  formBody.set("client_id", env.ONSHAPE_CLIENT_ID);
  formBody.set("client_secret", env.ONSHAPE_CLIENT_SECRET);
  formBody.set("redirect_uri", redirect_uri);
  if (typeof code_verifier === "string" && code_verifier !== "") {
    formBody.set("code_verifier", code_verifier);
  }

  return { type: "forward", url: ONSHAPE_TOKEN_URL, formBody };
}

function handleRefresh(body: unknown, env: Env): Effect {
  if (!isObject(body)) {
    return jsonError(400, "request body must be a JSON object");
  }

  const { refresh_token } = body as Record<string, unknown>;

  if (typeof refresh_token !== "string" || refresh_token === "") {
    return jsonError(400, "missing or invalid 'refresh_token' field");
  }

  const formBody = new URLSearchParams();
  formBody.set("grant_type", "refresh_token");
  formBody.set("refresh_token", refresh_token);
  formBody.set("client_id", env.ONSHAPE_CLIENT_ID);
  formBody.set("client_secret", env.ONSHAPE_CLIENT_SECRET);

  return { type: "forward", url: ONSHAPE_TOKEN_URL, formBody };
}

// ============================================================================
// Main Router
// ============================================================================

/**
 * Pure request router.
 *
 * @param ctx       Parsed request context.
 * @param env       Worker environment bindings (secrets).
 * @param allowedIps Pre-resolved list of all allowed IPs (direct + DNS-resolved).
 * @returns An Effect describing the response or upstream request to make.
 */
export function handleRequest(
  ctx: RequestContext,
  env: Env,
  allowedIps: string[],
): Effect {
  // Health check — exempt from IP restriction.
  if (ctx.pathname === "/health" && ctx.method === "GET") {
    return handleHealth();
  }

  // Config endpoint — exempt from IP restriction.
  if (ctx.pathname === "/config" && ctx.method === "GET") {
    return handleConfig(env);
  }

  // Method check for exempt endpoints that used the wrong method.
  if (ctx.pathname === "/health" || ctx.pathname === "/config") {
    return jsonError(405, "method_not_allowed");
  }

  // All remaining endpoints require ALLOWED_SOURCES to be configured.
  if (env.ALLOWED_SOURCES === undefined || env.ALLOWED_SOURCES === "") {
    return jsonError(500, "server misconfigured");
  }

  // IP restriction for all remaining endpoints.
  if (!isIpAllowed(ctx.sourceIp, allowedIps)) {
    return {
      type: "json-response",
      status: 403,
      body: { error: "forbidden", source_ip: ctx.sourceIp },
    };
  }

  // Token exchange.
  if (ctx.pathname === "/token/exchange") {
    if (ctx.method !== "POST") return jsonError(405, "method_not_allowed");
    return handleExchange(ctx.body, env);
  }

  // Token refresh.
  if (ctx.pathname === "/token/refresh") {
    if (ctx.method !== "POST") return jsonError(405, "method_not_allowed");
    return handleRefresh(ctx.body, env);
  }

  return jsonError(404, "not_found");
}

// ============================================================================
// Helpers
// ============================================================================

function jsonError(status: number, error: string): Effect {
  return { type: "json-response", status, body: { error } };
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
