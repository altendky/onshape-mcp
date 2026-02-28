/**
 * DNS-over-HTTPS hostname resolution via Cloudflare's public resolver.
 *
 * This is the only I/O in the access-restriction path.  Resolves hostnames
 * to IP addresses so the pure handler can do a simple set-membership check.
 */

/** Cloudflare DNS-over-HTTPS endpoint (JSON wire format). */
const DOH_URL = "https://cloudflare-dns.com/dns-query";

/** Shape of the JSON DNS response from Cloudflare DoH. */
interface DnsResponse {
  Answer?: Array<{ type: number; data: string }>;
}

/**
 * Resolve a list of hostnames to IP addresses (A + AAAA records).
 *
 * Queries are made in parallel.  Resolution failures for individual
 * hostnames are silently ignored — the hostname simply contributes
 * zero IPs to the allowed set (fail-closed).
 */
export async function resolveHostnames(hostnames: string[]): Promise<string[]> {
  if (hostnames.length === 0) return [];

  const queries = hostnames.flatMap((hostname) => [
    resolveType(hostname, "A"),
    resolveType(hostname, "AAAA"),
  ]);

  const results = await Promise.all(queries);
  return results.flat();
}

/** Query a single hostname for a single record type. */
async function resolveType(
  hostname: string,
  type: "A" | "AAAA",
): Promise<string[]> {
  try {
    const url = `${DOH_URL}?name=${encodeURIComponent(hostname)}&type=${type}`;
    const response = await fetch(url, {
      headers: { Accept: "application/dns-json" },
    });

    if (!response.ok) return [];

    const data = (await response.json()) as DnsResponse;

    // type 1 = A, type 28 = AAAA
    const expectedType = type === "A" ? 1 : 28;
    return (data.Answer ?? [])
      .filter((record) => record.type === expectedType)
      .map((record) => record.data);
  } catch {
    return [];
  }
}
