/** Cloudflare Worker environment bindings. */
export interface Env {
  /** Onshape OAuth application client ID. */
  ONSHAPE_CLIENT_ID: string;
  /** Onshape OAuth application client secret (encrypted secret). */
  ONSHAPE_CLIENT_SECRET: string;
  /** Comma-separated list of allowed source IPs and/or hostnames. */
  ALLOWED_SOURCES: string;
}

/** Parsed request context — extracted from the incoming Request in the I/O layer. */
export interface RequestContext {
  method: string;
  pathname: string;
  body: unknown;
  sourceIp: string;
}

/** Onshape OAuth token endpoint URL. */
export const ONSHAPE_TOKEN_URL = "https://oauth.onshape.com/oauth/token";

// ============================================================================
// Effects
// ============================================================================

/** Return a JSON response directly (errors, health, config). */
export interface JsonResponseEffect {
  type: "json-response";
  status: number;
  body: Record<string, unknown>;
}

/** Forward a form-encoded POST to Onshape's token endpoint. */
export interface ForwardEffect {
  type: "forward";
  url: string;
  formBody: URLSearchParams;
}

/** Discriminated union of all effects the pure handler can produce. */
export type Effect = JsonResponseEffect | ForwardEffect;

// ============================================================================
// Request body types
// ============================================================================

/** Expected body for POST /token/exchange. */
export interface ExchangeRequestBody {
  code: string;
  redirect_uri: string;
  code_verifier?: string;
}

/** Expected body for POST /token/refresh. */
export interface RefreshRequestBody {
  refresh_token: string;
}

// ============================================================================
// Allowed sources
// ============================================================================

/** Parsed ALLOWED_SOURCES — IPs are compared directly, hostnames need DNS resolution. */
export interface AllowedSources {
  ips: string[];
  hostnames: string[];
}
