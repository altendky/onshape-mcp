/**
 * Cloudflare Worker entry point — thin I/O executor.
 *
 * 1. Parses the incoming Request into a RequestContext.
 * 2. Resolves hostnames from ALLOWED_SOURCES via DNS-over-HTTPS.
 * 3. Calls the pure handler to get an Effect.
 * 4. Executes the Effect (returns JSON or forwards to Onshape).
 */

import { resolveHostnames } from "./dns.js";
import { handleRequest, parseAllowedSources } from "./handler.js";
import type { Effect, Env, RequestContext } from "./types.js";

const JSON_HEADERS = { "Content-Type": "application/json" };

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // 1. Parse request context.
    const url = new URL(request.url);
    const ctx: RequestContext = {
      method: request.method,
      pathname: url.pathname,
      body: await parseJsonBody(request),
      sourceIp: request.headers.get("CF-Connecting-IP") ?? "",
    };

    // 2. Resolve allowed IPs (direct IPs + DNS-resolved hostnames).
    const sources = parseAllowedSources(env.ALLOWED_SOURCES ?? "");
    const resolvedIps = await resolveHostnames(sources.hostnames);
    const allAllowedIps = [...sources.ips, ...resolvedIps];

    // 3. Run pure handler.
    const effect = handleRequest(ctx, env, allAllowedIps);

    // 4. Execute effect.
    return executeEffect(effect);
  },
};

// ============================================================================
// Effect Executor
// ============================================================================

async function executeEffect(effect: Effect): Promise<Response> {
  switch (effect.type) {
    case "json-response":
      return new Response(JSON.stringify(effect.body), {
        status: effect.status,
        headers: JSON_HEADERS,
      });

    case "forward": {
      const upstream = await fetch(effect.url, {
        method: "POST",
        body: effect.formBody,
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
      });

      // Always return JSON.  Onshape's token endpoint normally returns
      // JSON, but if it ever returns something else (HTML error page,
      // empty body, etc.) we wrap it so clients can rely on JSON.
      const upstreamBody = await upstream.text();
      let jsonBody: string;
      try {
        // Validate it's actually JSON by parsing, then pass through
        // the original text to avoid any re-serialisation changes.
        JSON.parse(upstreamBody);
        jsonBody = upstreamBody;
      } catch {
        // Not JSON — wrap the raw body in a JSON error envelope.
        jsonBody = JSON.stringify({
          error: "upstream_error",
          upstream_status: upstream.status,
          upstream_body: upstreamBody,
        });
      }

      return new Response(jsonBody, {
        status: upstream.status,
        headers: JSON_HEADERS,
      });
    }
  }
}

// ============================================================================
// Helpers
// ============================================================================

/** Try to parse the request body as JSON; return null on failure. */
async function parseJsonBody(request: Request): Promise<unknown> {
  if (request.method === "GET" || request.method === "HEAD") return null;
  try {
    return await request.json();
  } catch {
    return null;
  }
}
