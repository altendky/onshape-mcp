import { describe, expect, it } from "vitest";

import {
  handleRequest,
  isIpAllowed,
  parseAllowedSources,
} from "../src/handler.js";
import type { Env, ForwardEffect, JsonResponseEffect, RequestContext } from "../src/types.js";
import { ONSHAPE_TOKEN_URL } from "../src/types.js";

// ============================================================================
// Test Helpers
// ============================================================================

const TEST_ENV: Env = {
  ONSHAPE_CLIENT_ID: "test-client-id",
  ONSHAPE_CLIENT_SECRET: "test-client-secret",
  ALLOWED_SOURCES: "10.0.0.1,home.example.com",
};

const ALLOWED_IP = "10.0.0.1";
const DISALLOWED_IP = "192.168.1.99";

function ctx(
  overrides: Partial<RequestContext> = {},
): RequestContext {
  return {
    method: "GET",
    pathname: "/health",
    body: null,
    sourceIp: ALLOWED_IP,
    ...overrides,
  };
}

function asJson(effect: JsonResponseEffect): Record<string, unknown> {
  return effect.body;
}

// ============================================================================
// parseAllowedSources
// ============================================================================

describe("parseAllowedSources", () => {
  it("classifies IPv4 addresses as ips", () => {
    const result = parseAllowedSources("10.0.0.1,192.168.1.1");
    expect(result.ips).toEqual(["10.0.0.1", "192.168.1.1"]);
    expect(result.hostnames).toEqual([]);
  });

  it("classifies IPv6 addresses as ips", () => {
    const result = parseAllowedSources("::1,2001:db8::1");
    expect(result.ips).toEqual(["::1", "2001:db8::1"]);
    expect(result.hostnames).toEqual([]);
  });

  it("classifies hostnames as hostnames", () => {
    const result = parseAllowedSources("home.example.com,vpn.corp.net");
    expect(result.hostnames).toEqual(["home.example.com", "vpn.corp.net"]);
    expect(result.ips).toEqual([]);
  });

  it("handles mixed list", () => {
    const result = parseAllowedSources("10.0.0.1,home.example.com,::1");
    expect(result.ips).toEqual(["10.0.0.1", "::1"]);
    expect(result.hostnames).toEqual(["home.example.com"]);
  });

  it("handles empty string", () => {
    const result = parseAllowedSources("");
    expect(result.ips).toEqual([]);
    expect(result.hostnames).toEqual([]);
  });

  it("trims whitespace", () => {
    const result = parseAllowedSources("  10.0.0.1 , home.example.com  ");
    expect(result.ips).toEqual(["10.0.0.1"]);
    expect(result.hostnames).toEqual(["home.example.com"]);
  });

  it("skips empty entries from trailing commas", () => {
    const result = parseAllowedSources("10.0.0.1,,home.example.com,");
    expect(result.ips).toEqual(["10.0.0.1"]);
    expect(result.hostnames).toEqual(["home.example.com"]);
  });
});

// ============================================================================
// isIpAllowed
// ============================================================================

describe("isIpAllowed", () => {
  it("returns true for allowed IP", () => {
    expect(isIpAllowed("10.0.0.1", ["10.0.0.1", "192.168.1.1"])).toBe(true);
  });

  it("returns false for disallowed IP", () => {
    expect(isIpAllowed("172.16.0.1", ["10.0.0.1", "192.168.1.1"])).toBe(false);
  });

  it("returns false for empty allowed list", () => {
    expect(isIpAllowed("10.0.0.1", [])).toBe(false);
  });

  it("matches equivalent IPv6 representations", () => {
    // Compact vs. expanded — same address, different text.
    expect(isIpAllowed("::1", ["0:0:0:0:0:0:0:1"])).toBe(true);
    expect(isIpAllowed("2001:db8::1", ["2001:0db8:0000:0000:0000:0000:0000:0001"])).toBe(true);
  });

  it("matches IPv6 with leading-zero differences", () => {
    expect(isIpAllowed("2001:db8::1", ["2001:0db8::0001"])).toBe(true);
  });

  it("rejects different IPv6 addresses despite similar text", () => {
    expect(isIpAllowed("::1", ["::2"])).toBe(false);
  });
});

// ============================================================================
// Routing: GET /health
// ============================================================================

describe("GET /health", () => {
  it("returns status ok", () => {
    const effect = handleRequest(ctx(), TEST_ENV, [ALLOWED_IP]);
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(200);
    expect(asJson(effect as JsonResponseEffect)).toEqual({ status: "ok" });
  });

  it("is exempt from IP restriction", () => {
    const effect = handleRequest(
      ctx({ sourceIp: DISALLOWED_IP }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(200);
  });

  it("rejects non-GET methods", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/health" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(405);
  });
});

// ============================================================================
// Routing: GET /config
// ============================================================================

describe("GET /config", () => {
  it("returns client_id", () => {
    const effect = handleRequest(
      ctx({ pathname: "/config" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    const json = effect as JsonResponseEffect;
    expect(json.status).toBe(200);
    expect(json.body).toEqual({ client_id: "test-client-id" });
  });

  it("is blocked for disallowed IP", () => {
    const effect = handleRequest(
      ctx({ pathname: "/config", sourceIp: DISALLOWED_IP }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(403);
    expect((effect as JsonResponseEffect).body).toEqual({
      error: "forbidden",
      source_ip: DISALLOWED_IP,
    });
  });

  it("rejects non-GET methods", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/config" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(405);
  });
});

// ============================================================================
// IP Restriction
// ============================================================================

describe("IP restriction", () => {
  it("blocks disallowed IP on /config", () => {
    const effect = handleRequest(
      ctx({
        pathname: "/config",
        sourceIp: DISALLOWED_IP,
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(403);
    expect((effect as JsonResponseEffect).body).toEqual({
      error: "forbidden",
      source_ip: DISALLOWED_IP,
    });
  });

  it("blocks disallowed IP on /token/exchange", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/exchange",
        sourceIp: DISALLOWED_IP,
        body: { code: "abc", redirect_uri: "http://localhost:8080/callback" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(403);
    expect((effect as JsonResponseEffect).body).toEqual({
      error: "forbidden",
      source_ip: DISALLOWED_IP,
    });
  });

  it("blocks disallowed IP on /token/refresh", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/refresh",
        sourceIp: DISALLOWED_IP,
        body: { refresh_token: "rt" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(403);
    expect((effect as JsonResponseEffect).body).toEqual({
      error: "forbidden",
      source_ip: DISALLOWED_IP,
    });
  });
});

// ============================================================================
// Server Misconfiguration
// ============================================================================

describe("ALLOWED_SOURCES misconfigured", () => {
  it("returns 500 when ALLOWED_SOURCES is empty", () => {
    const envNoSources: Env = { ...TEST_ENV, ALLOWED_SOURCES: "" };
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/exchange", body: { code: "x", redirect_uri: "y" } }),
      envNoSources,
      [],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(500);
    expect((effect as JsonResponseEffect).body).toEqual({ error: "server misconfigured" });
  });

  it("does not affect /health", () => {
    const envNoSources: Env = { ...TEST_ENV, ALLOWED_SOURCES: "" };
    const effect = handleRequest(ctx(), envNoSources, []);
    expect((effect as JsonResponseEffect).status).toBe(200);
  });

  it("blocks /config when ALLOWED_SOURCES is empty", () => {
    const envNoSources: Env = { ...TEST_ENV, ALLOWED_SOURCES: "" };
    const effect = handleRequest(
      ctx({ pathname: "/config" }),
      envNoSources,
      [],
    );
    expect((effect as JsonResponseEffect).status).toBe(500);
    expect((effect as JsonResponseEffect).body).toEqual({ error: "server misconfigured" });
  });
});

// ============================================================================
// POST /token/exchange
// ============================================================================

describe("POST /token/exchange", () => {
  const validBody = {
    code: "auth-code-123",
    redirect_uri: "http://localhost:18338/callback",
  };

  it("returns a forward effect with correct form body", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/exchange", body: validBody }),
      TEST_ENV,
      [ALLOWED_IP],
    );

    expect(effect.type).toBe("forward");
    const fwd = effect as ForwardEffect;
    expect(fwd.url).toBe(ONSHAPE_TOKEN_URL);
    expect(fwd.formBody.get("grant_type")).toBe("authorization_code");
    expect(fwd.formBody.get("code")).toBe("auth-code-123");
    expect(fwd.formBody.get("client_id")).toBe("test-client-id");
    expect(fwd.formBody.get("client_secret")).toBe("test-client-secret");
    expect(fwd.formBody.get("redirect_uri")).toBe("http://localhost:18338/callback");
    expect(fwd.formBody.has("code_verifier")).toBe(false);
  });

  it("includes code_verifier when provided", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/exchange",
        body: { ...validBody, code_verifier: "pkce-verifier-abc" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );

    expect(effect.type).toBe("forward");
    const fwd = effect as ForwardEffect;
    expect(fwd.formBody.get("code_verifier")).toBe("pkce-verifier-abc");
  });

  it("rejects missing code", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/exchange",
        body: { redirect_uri: "http://localhost:8080/callback" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(400);
    expect((effect as JsonResponseEffect).body.error).toMatch(/code/);
  });

  it("rejects missing redirect_uri", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/exchange",
        body: { code: "abc" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
    expect((effect as JsonResponseEffect).body.error).toMatch(/redirect_uri/);
  });

  it("rejects non-object body", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/exchange", body: "not-json" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
  });

  it("rejects null body", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/exchange", body: null }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
  });

  it("rejects GET method", () => {
    const effect = handleRequest(
      ctx({ method: "GET", pathname: "/token/exchange" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(405);
  });

  it("credentials come from env, not from request body", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/exchange",
        body: {
          ...validBody,
          client_id: "attacker-id",
          client_secret: "attacker-secret",
        },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );

    const fwd = effect as ForwardEffect;
    expect(fwd.formBody.get("client_id")).toBe("test-client-id");
    expect(fwd.formBody.get("client_secret")).toBe("test-client-secret");
  });
});

// ============================================================================
// POST /token/refresh
// ============================================================================

describe("POST /token/refresh", () => {
  const validBody = { refresh_token: "rt-abc-123" };

  it("returns a forward effect with correct form body", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/refresh", body: validBody }),
      TEST_ENV,
      [ALLOWED_IP],
    );

    expect(effect.type).toBe("forward");
    const fwd = effect as ForwardEffect;
    expect(fwd.url).toBe(ONSHAPE_TOKEN_URL);
    expect(fwd.formBody.get("grant_type")).toBe("refresh_token");
    expect(fwd.formBody.get("refresh_token")).toBe("rt-abc-123");
    expect(fwd.formBody.get("client_id")).toBe("test-client-id");
    expect(fwd.formBody.get("client_secret")).toBe("test-client-secret");
  });

  it("rejects missing refresh_token", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/refresh", body: {} }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
    expect((effect as JsonResponseEffect).body.error).toMatch(/refresh_token/);
  });

  it("rejects empty refresh_token", () => {
    const effect = handleRequest(
      ctx({
        method: "POST",
        pathname: "/token/refresh",
        body: { refresh_token: "" },
      }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
  });

  it("rejects non-object body", () => {
    const effect = handleRequest(
      ctx({ method: "POST", pathname: "/token/refresh", body: null }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(400);
  });

  it("rejects GET method", () => {
    const effect = handleRequest(
      ctx({ method: "GET", pathname: "/token/refresh" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(405);
  });
});

// ============================================================================
// Unknown Routes
// ============================================================================

describe("unknown routes", () => {
  it("returns 404 for unknown path", () => {
    const effect = handleRequest(
      ctx({ pathname: "/unknown" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect(effect.type).toBe("json-response");
    expect((effect as JsonResponseEffect).status).toBe(404);
    expect((effect as JsonResponseEffect).body).toEqual({ error: "not_found" });
  });

  it("returns 404 for root path", () => {
    const effect = handleRequest(
      ctx({ pathname: "/" }),
      TEST_ENV,
      [ALLOWED_IP],
    );
    expect((effect as JsonResponseEffect).status).toBe(404);
  });
});
