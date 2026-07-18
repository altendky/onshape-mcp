import { describe, expect, test } from "bun:test";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  rmdirSync,
} from "fs";
import { tmpdir } from "os";
import { join } from "path";

import {
  OnshapeAuthPlugin,
  publishTokenFile,
  resolveDataDir,
  saveTokens,
  tokenLockPath,
  validateProxyUrl,
  withTokenFileLock,
} from "./plugin.js";

async function methods() {
  const plugin = await OnshapeAuthPlugin({} as never);
  return plugin.auth!.methods;
}

describe("Onshape auth methods", () => {
  test("honors an absolute POSIX XDG data override", () => {
    expect(
      resolveDataDir("darwin", "/home/test", {
        XDG_DATA_HOME: "/isolated/data",
      }),
    ).toBe("/isolated/data/onshape-mcp");
  });

  test("honors only drive-qualified or UNC Windows XDG data overrides", () => {
    const localData = "C:\\Users\\test\\AppData\\Local";
    const resolveWindowsDataDir = (xdgDataHome: string) =>
      resolveDataDir("win32", "C:\\Users\\test", {
        XDG_DATA_HOME: xdgDataHome,
        LOCALAPPDATA: localData,
      });

    expect(resolveWindowsDataDir("C:\\isolated\\data")).toBe(
      "C:\\isolated\\data\\onshape-mcp",
    );
    expect(resolveWindowsDataDir("\\\\server\\share\\data")).toBe(
      "\\\\server\\share\\data\\onshape-mcp",
    );
    expect(resolveWindowsDataDir("\\data")).toBe(
      `${localData}\\onshape-mcp`,
    );
    expect(resolveWindowsDataDir("/data")).toBe(
      `${localData}\\onshape-mcp`,
    );
    expect(resolveWindowsDataDir("relative")).toBe(
      `${localData}\\onshape-mcp`,
    );
  });

  test("APPDATA cannot override LOCALAPPDATA for Windows token storage", () => {
    expect(
      resolveDataDir("win32", "/home/test", {
        LOCALAPPDATA: "C:\\Users\\test\\AppData\\Local",
        APPDATA: "C:\\Users\\test\\AppData\\Roaming",
      }),
    ).toBe("C:\\Users\\test\\AppData\\Local\\onshape-mcp");
  });

  test("offers direct OAuth first and labels proxy as self-hosted", async () => {
    const authMethods = await methods();

    expect(authMethods).toHaveLength(2);
    expect(authMethods[0]!.label).toBe("Onshape OAuth (direct)");
    expect(authMethods[1]!.label).toBe("Onshape OAuth (self-hosted proxy)");
  });

  test("proxy URL validation matches loopback policy", () => {
    expect(
      validateProxyUrl("https://proxy.example.com/base/?query=yes#fragment"),
    ).toBe("https://proxy.example.com/base");
    expect(validateProxyUrl("http://localhost:8787")).toBe(
      "http://localhost:8787",
    );
    expect(validateProxyUrl("http://127.42.3.9:8787/")).toBe(
      "http://127.42.3.9:8787",
    );
    expect(validateProxyUrl("http://[::1]:8787")).toBe("http://[::1]:8787");
    expect(() => validateProxyUrl("http://proxy.example.com")).toThrow(
      "must use https://",
    );
    expect(() => validateProxyUrl("http://128.0.0.1")).toThrow(
      "must use https://",
    );
    expect(() => validateProxyUrl("  ")).toThrow("is required");
    expect(() => validateProxyUrl("not a URL")).toThrow();
  });

  test("uses only a placeholder proxy URL", async () => {
    const authMethods = await methods();
    const proxy = authMethods[1]!;

    expect(proxy.prompts?.[0]?.placeholder).toBe(
      "https://oauth-proxy.example.com",
    );
    await expect(proxy.authorize?.({ proxy_url: "" })).rejects.toThrow(
      "Self-hosted OAuth Proxy URL is required",
    );
  });

  test("rejects non-HTTPS non-local proxy URLs", async () => {
    const authMethods = await methods();
    const proxy = authMethods[1]!;

    await expect(
      proxy.authorize?.({ proxy_url: "http://oauth-proxy.example.com" }),
    ).rejects.toThrow("must use https://");
  });

  test("still requires direct client credentials", async () => {
    const authMethods = await methods();
    const direct = authMethods[0]!;

    await expect(direct.authorize?.({})).rejects.toThrow(
      "Client ID and Client Secret are required",
    );
  });

  test("rejects whitespace-only direct client credentials", async () => {
    const authMethods = await methods();
    const direct = authMethods[0]!;

    await expect(
      direct.authorize?.({ client_id: "  ", client_secret: "secret" }),
    ).rejects.toThrow("Client ID and Client Secret are required");
    await expect(
      direct.authorize?.({ client_id: "client", client_secret: "\t " }),
    ).rejects.toThrow("Client ID and Client Secret are required");
  });
});

const testTokens = {
  access_token: "access",
  refresh_token: "refresh",
  expires_at: "2099-01-01T00:00:00Z",
  token_type: "bearer",
  scopes: ["OAuth2Read"],
  client_id: "client",
  client_secret: "secret",
};

describe("token writer lock", () => {
  test("bounds persistent Windows sharing failures and rethrows the final error", async () => {
    const publicationError = Object.assign(new Error("file is still shared"), {
      code: "EACCES",
    });
    const timeoutMs = 50;
    const started = Bun.nanoseconds();
    let attempts = 0;
    let thrown: unknown;

    try {
      await publishTokenFile(
        () => {
          attempts += 1;
          throw publicationError;
        },
        "win32",
        timeoutMs,
      );
    } catch (error) {
      thrown = error;
    }

    const elapsedMs = (Bun.nanoseconds() - started) / 1_000_000;
    expect(thrown).toBe(publicationError);
    expect(attempts).toBeGreaterThan(1);
    expect(elapsedMs).toBeGreaterThanOrEqual(timeoutMs);
    expect(elapsedMs).toBeLessThan(500);
  });

  test("holds the shared lock for the complete operation", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-transaction-"));
    const path = join(dir, "tokens.json");
    const lockPath = tokenLockPath(path);
    try {
      await withTokenFileLock(async () => {
        expect(statSync(lockPath).isDirectory()).toBe(true);
        expect(existsSync(path)).toBe(false);
      }, path);
      expect(existsSync(lockPath)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("does not start an exchange operation while another writer holds the lock", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-exchange-wait-"));
    const path = join(dir, "tokens.json");
    const lockPath = tokenLockPath(path);
    let exchangeStarted = false;
    try {
      mkdirSync(lockPath);
      const transaction = withTokenFileLock(async () => {
        exchangeStarted = true;
      }, path);
      await Bun.sleep(75);
      expect(exchangeStarted).toBe(false);
      rmdirSync(lockPath);
      await transaction;
      expect(exchangeStarted).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("releases the transaction lock after an exchange error", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-exchange-error-"));
    const path = join(dir, "tokens.json");
    try {
      await expect(
        withTokenFileLock(async () => {
          throw new Error("exchange failed");
        }, path),
      ).rejects.toThrow("exchange failed");
      expect(existsSync(tokenLockPath(path))).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("uses the shared adjacent lock name and cleans it up", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-lock-"));
    const path = join(dir, "tokens.json");
    try {
      expect(tokenLockPath(path)).toBe(`${path}.lock`);
      await saveTokens(testTokens, path);
      expect(existsSync(tokenLockPath(path))).toBe(false);
      expect(JSON.parse(readFileSync(path, "utf8")).access_token).toBe(
        "access",
      );
      if (process.platform !== "win32") {
        expect(statSync(path).mode & 0o777).toBe(0o600);
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("waits for an active writer and then saves", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-wait-"));
    const path = join(dir, "tokens.json");
    const lockPath = tokenLockPath(path);
    try {
      mkdirSync(lockPath);
      const save = saveTokens({ ...testTokens, access_token: "login" }, path);
      await Bun.sleep(75);
      expect(existsSync(path)).toBe(false);
      rmdirSync(lockPath);
      await save;
      expect(JSON.parse(readFileSync(path, "utf8")).access_token).toBe("login");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("times out without deleting an abandoned lock directory", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-timeout-"));
    const path = join(dir, "tokens.json");
    const lockPath = tokenLockPath(path);
    try {
      mkdirSync(lockPath);
      await expect(saveTokens(testTokens, path, 10)).rejects.toThrow(
        `Timed out waiting for token file lock directory ${lockPath}. Stop all onshape-mcp/OpenCode writers, then manually remove the lock directory if no writer is running.`,
      );
      expect(statSync(lockPath).isDirectory()).toBe(true);
      expect(existsSync(path)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("cleans the lock after a write failure", async () => {
    const dir = mkdtempSync(join(tmpdir(), "onshape-token-failure-"));
    const path = join(dir, "tokens.json");
    try {
      mkdirSync(path);
      await expect(saveTokens(testTokens, path)).rejects.toThrow();
      expect(existsSync(tokenLockPath(path))).toBe(false);
      expect(readdirSync(dir)).toEqual(["tokens.json"]);
    } finally {
      chmodSync(dir, 0o700);
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
