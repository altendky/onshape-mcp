/**
 * Tests for the DNS-aware proxy request helpers — mirrors the Rust tests
 * in crates/onshape-mcp-io/src/lib.rs (ipv4_retry_* tests).
 *
 * These test the parsing/decision logic only, not actual network connections.
 */

import { describe, expect, test } from "bun:test";
import {
  isIpv6,
  parseProxyError,
  shouldRetryIpv4,
  tryAddresses,
} from "./ipv4-retry.js";

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
    expect(parseProxyError(JSON.stringify({ source_ip: "1.2.3.4" }))).toBeNull();
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
  test("returns null for empty address list (POST)", async () => {
    const result = await tryAddresses([], "https://example.com/test", {});
    expect(result).toBeNull();
  });

  test("returns null for empty address list (GET)", async () => {
    const result = await tryAddresses([], "https://example.com/test", null, "GET");
    expect(result).toBeNull();
  });

  // Note: tryAddresses with real addresses would require network access.
  // The connection-error-then-next-address logic is tested implicitly
  // by the function's contract: it catches errors and continues the loop.
});
