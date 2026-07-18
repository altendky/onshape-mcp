/**
 * Tests for the DNS-aware proxy request helpers — mirrors the Rust tests
 * in crates/onshape-mcp-io/src/lib.rs (ipv4_retry_* tests).
 *
 * These test the parsing/decision logic only, not actual network connections.
 */

import { describe, expect, test } from "bun:test";
import { createServer } from "node:http";
import { networkInterfaces } from "node:os";
import {
  isIpv6,
  parseProxyError,
  requestOptionsForIp,
  resolveIpv4,
  resolveIpv6,
  shouldRetryIpv4,
  tryAddresses,
} from "./ipv4-retry.js";

const hasIpv6Loopback = Object.values(networkInterfaces()).some((addresses) =>
  addresses?.some(({ address }) => address === "::1"),
);

async function localHttpServer(host: string) {
  const requests: Array<{ method?: string; host?: string; body: string }> = [];
  const server = createServer((request, response) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      requests.push({
        method: request.method,
        host: request.headers.host,
        body,
      });
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end(JSON.stringify({ method: request.method, body }));
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, host, resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("missing server address");
  return { server, requests, port: address.port };
}

async function closeServer(server: ReturnType<typeof createServer>) {
  await new Promise<void>((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

async function localResponseServer(body: string) {
  let requestCount = 0;
  const server = createServer((_request, response) => {
    requestCount += 1;
    response.writeHead(200, { "Content-Type": "text/plain" });
    response.end(body);
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("missing server address");
  return {
    server,
    port: address.port,
    get requestCount() {
      return requestCount;
    },
  };
}

// ============================================================================
// parseProxyError
// ============================================================================

describe("parseProxyError", () => {
  test("parses a standard 403 response with source_ip", () => {
    const body = JSON.stringify({
      error: "forbidden",
      source_ip: "2601:980:c200:8530:bfc8:c956:e7c1:1d07",
    });
    expect(parseProxyError(body)).toEqual({
      error: "forbidden",
      source_ip: "2601:980:c200:8530:bfc8:c956:e7c1:1d07",
    });
  });

  test("parses error without source_ip", () => {
    const body = JSON.stringify({ error: "forbidden" });
    expect(parseProxyError(body)).toEqual({ error: "forbidden" });
  });

  test("returns null for non-JSON", () => {
    expect(parseProxyError("not json")).toBeNull();
  });

  test("returns null for empty string", () => {
    expect(parseProxyError("")).toBeNull();
  });

  test("returns null for JSON without error field", () => {
    expect(
      parseProxyError(JSON.stringify({ source_ip: "1.2.3.4" })),
    ).toBeNull();
  });

  test("returns null for JSON array", () => {
    expect(parseProxyError("[]")).toBeNull();
  });

  test("returns null for JSON number", () => {
    expect(parseProxyError("42")).toBeNull();
  });

  test("omits source_ip when it is empty string", () => {
    const body = JSON.stringify({ error: "forbidden", source_ip: "" });
    expect(parseProxyError(body)).toEqual({ error: "forbidden" });
  });

  test("omits source_ip when it is not a string", () => {
    const body = JSON.stringify({ error: "forbidden", source_ip: 12345 });
    expect(parseProxyError(body)).toEqual({ error: "forbidden" });
  });

  test("parses IPv4 source_ip", () => {
    const body = JSON.stringify({
      error: "forbidden",
      source_ip: "71.58.134.128",
    });
    expect(parseProxyError(body)).toEqual({
      error: "forbidden",
      source_ip: "71.58.134.128",
    });
  });
});

// ============================================================================
// isIpv6
// ============================================================================

describe("isIpv6", () => {
  test("returns true for full IPv6 address", () => {
    expect(isIpv6("2601:980:c200:8530:bfc8:c956:e7c1:1d07")).toBe(true);
  });

  test("returns true for IPv6 loopback", () => {
    expect(isIpv6("::1")).toBe(true);
  });

  test("returns true for IPv6 unspecified", () => {
    expect(isIpv6("::")).toBe(true);
  });

  test("returns false for IPv4 address", () => {
    expect(isIpv6("71.58.134.128")).toBe(false);
  });

  test("returns false for IPv4 loopback", () => {
    expect(isIpv6("127.0.0.1")).toBe(false);
  });
});

// ============================================================================
// shouldRetryIpv4
// ============================================================================

describe("shouldRetryIpv4", () => {
  test("returns true for IPv6 source", () => {
    const body = JSON.stringify({
      error: "forbidden",
      source_ip: "2601:980:c200:8530:bfc8:c956:e7c1:1d07",
    });
    expect(shouldRetryIpv4(body)).toBe(true);
  });

  test("returns false for IPv4 source", () => {
    const body = JSON.stringify({
      error: "forbidden",
      source_ip: "71.58.134.128",
    });
    expect(shouldRetryIpv4(body)).toBe(false);
  });

  test("returns false for unparsable body", () => {
    expect(shouldRetryIpv4("not json")).toBe(false);
  });

  test("returns false for missing source_ip", () => {
    const body = JSON.stringify({ error: "forbidden" });
    expect(shouldRetryIpv4(body)).toBe(false);
  });

  test("returns true for IPv6 loopback", () => {
    const body = JSON.stringify({
      error: "forbidden",
      source_ip: "::1",
    });
    expect(shouldRetryIpv4(body)).toBe(true);
  });

  test("returns false for empty body", () => {
    expect(shouldRetryIpv4("")).toBe(false);
  });
});

// ============================================================================
// tryAddresses
// ============================================================================

describe("tryAddresses", () => {
  test("does not start another request after the transaction deadline", async () => {
    const result = await tryAddresses(
      ["127.0.0.1"],
      "http://example.com/test",
      {},
      "POST",
      Date.now() - 1,
    );
    expect(result).toBeNull();
  });

  test("returns null for empty address list (POST)", async () => {
    const result = await tryAddresses([], "https://example.com/test", {});
    expect(result).toBeNull();
  });

  test("returns null for empty address list (GET)", async () => {
    const result = await tryAddresses(
      [],
      "https://example.com/test",
      null,
      "GET",
    );
    expect(result).toBeNull();
  });

  test("performs local HTTP GET with the original localhost Host", async () => {
    const { server, requests, port } = await localHttpServer("127.0.0.1");
    try {
      const result = await tryAddresses(
        ["127.0.0.1"],
        `http://localhost:${port}/config?test=yes`,
        null,
        "GET",
      );
      expect(result?.status).toBe(200);
      expect(requests).toEqual([
        {
          method: "GET",
          host: `localhost:${port}`,
          body: "",
        },
      ]);
    } finally {
      await closeServer(server);
    }
  });

  test("performs local HTTP POST to an IPv4 literal", async () => {
    const { server, requests, port } = await localHttpServer("127.0.0.1");
    try {
      const result = await tryAddresses(
        ["127.0.0.1"],
        `http://127.0.0.1:${port}/token/exchange`,
        { code: "test" },
      );
      expect(result?.status).toBe(200);
      expect(requests).toEqual([
        {
          method: "POST",
          host: `127.0.0.1:${port}`,
          body: JSON.stringify({ code: "test" }),
        },
      ]);
    } finally {
      await closeServer(server);
    }
  });

  test("accepts a proxy response at the byte limit", async () => {
    const body = "é".repeat(512 * 1024);
    const local = await localResponseServer(body);
    try {
      const result = await tryAddresses(
        ["127.0.0.1"],
        `http://localhost:${local.port}/config`,
        null,
        "GET",
      );
      expect(result?.body).toBe(body);
    } finally {
      await closeServer(local.server);
    }
  });

  test("rejects an oversized proxy response without retrying", async () => {
    const body = `${"é".repeat(512 * 1024)}x`;
    const local = await localResponseServer(body);
    try {
      await expect(
        tryAddresses(
          ["127.0.0.1", "127.0.0.1"],
          `http://localhost:${local.port}/config`,
          null,
          "GET",
        ),
      ).rejects.toThrow("proxy response body exceeds 1048576 byte limit");
      expect(local.requestCount).toBe(1);
    } finally {
      await closeServer(local.server);
    }
  });

  test.skipIf(!hasIpv6Loopback)(
    "performs local HTTP GET to an IPv6 literal when available",
    async () => {
      const local = await localHttpServer("::1");
      const { server, requests, port } = local;
      try {
        const result = await tryAddresses(
          ["::1"],
          `http://[::1]:${port}/config`,
          null,
          "GET",
        );
        expect(result?.status).toBe(200);
        expect(requests[0]?.host).toBe(`[::1]:${port}`);
      } finally {
        await closeServer(server);
      }
    },
  );
});

describe("address resolution", () => {
  test("handles localhost without DNS", async () => {
    expect(await resolveIpv4("localhost")).toEqual(["127.0.0.1"]);
    expect(await resolveIpv6("localhost")).toEqual(["::1"]);
  });

  test("handles IPv4 literals without DNS", async () => {
    expect(await resolveIpv4("127.42.3.9")).toEqual(["127.42.3.9"]);
    expect(await resolveIpv6("127.42.3.9")).toEqual([]);
  });

  test("handles bracketed IPv6 literals without DNS", async () => {
    expect(await resolveIpv4("[::1]")).toEqual([]);
    expect(await resolveIpv6("[::1]")).toEqual(["::1"]);
  });
});

describe("request options", () => {
  test("preserves HTTPS DNS SNI, Host, port, path, and query", () => {
    const options = requestOptionsForIp(
      "192.0.2.10",
      new URL("https://proxy.example.com:8443/base/config?test=yes"),
      "GET",
      null,
    );
    expect(options.hostname).toBe("192.0.2.10");
    expect(options.port).toBe("8443");
    expect(options.servername).toBe("proxy.example.com");
    expect(options.headers).toEqual({ Host: "proxy.example.com:8443" });
    expect(options.path).toBe("/base/config?test=yes");
  });

  test("omits SNI for HTTPS IP literals and uses protocol defaults", () => {
    const httpsOptions = requestOptionsForIp(
      "::1",
      new URL("https://[::1]/config"),
      "GET",
      null,
    );
    expect(httpsOptions.port).toBe(443);
    expect(httpsOptions.servername).toBeUndefined();
    expect(httpsOptions.headers).toEqual({ Host: "[::1]" });

    const httpOptions = requestOptionsForIp(
      "127.0.0.1",
      new URL("http://127.0.0.1/config"),
      "GET",
      null,
    );
    expect(httpOptions.port).toBe(80);
    expect(httpOptions.servername).toBeUndefined();
  });
});
