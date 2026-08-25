import type { R2Bucket, RateLimit } from "@cloudflare/workers-types";

export interface Env {
  ASSETS: Fetcher;
  PUBLICATIONS: R2Bucket;
  CREATE_ENABLED?: string;
  CREATE_LIMITER?: RateLimit;
  GET_LIMITER?: RateLimit;
  REVOKE_LIMITER?: RateLimit;
}

const MAX_ENVELOPE_BYTES = 5 * 1024 * 1024;
const MAGIC = new TextEncoder().encode("SIVTPUB1");
const ID_RE = /^(1d|7d|30d|90d)_([A-Za-z0-9_-]{22})$/;

type ExpiryClass = "1d" | "7d" | "30d" | "90d";

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const started = Date.now();
    let response: Response;
    try {
      response = await route(request, env, ctx);
    } catch (error) {
      logEvent("error", request, 0, Date.now() - started, errorCategory(error));
      response = json({ error: "internal_error" }, 500);
    }
    logEvent("request", request, response.status, Date.now() - started, undefined, response.headers.get("content-length"));
    return withSecurityHeaders(response);
  },
};

async function route(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  const apiMatch = url.pathname.match(/^\/api\/v1\/publications\/([^/]+)$/);
  if (apiMatch) {
    const id = apiMatch[1];
    return api(request, env, ctx, id);
  }
  if (request.method !== "GET") return json({ error: "not_found" }, 404);
  // `/s/:id` is intentionally a fixed shell; the server never inspects or
  // renders the encrypted snapshot, including its title.
  if (url.pathname === "/" || url.pathname.startsWith("/s/")) {
    return env.ASSETS.fetch(new Request(new URL("/index.html", request.url), request));
  }
  return env.ASSETS.fetch(request);
}

async function api(request: Request, env: Env, ctx: ExecutionContext, id: string): Promise<Response> {
  const parsed = parseId(id);
  if (!parsed) return json({ error: "not_found" }, 404);
  if (request.method === "PUT") return put(request, env, parsed);
  if (request.method === "GET") return get(request, env, parsed);
  if (request.method === "DELETE") return remove(request, env, parsed);
  return json({ error: "method_not_allowed" }, 405, { Allow: "GET, PUT, DELETE" });
}

function parseId(id: string): { id: string; expiry: ExpiryClass; random: string; key: string } | null {
  const match = ID_RE.exec(id);
  if (!match) return null;
  return { id, expiry: match[1] as ExpiryClass, random: match[2], key: `v1/${match[1]}/${match[2]}` };
}

async function put(request: Request, env: Env, publication: NonNullable<ReturnType<typeof parseId>>): Promise<Response> {
  if (env.CREATE_ENABLED === "false") return json({ error: "creation_disabled" }, 503);
  if (await limited(env.CREATE_LIMITER, clientIp(request))) return json({ error: "rate_limited" }, 429);
  const declaredLength = Number(request.headers.get("content-length") ?? "0");
  if (declaredLength > MAX_ENVELOPE_BYTES) return json({ error: "payload_too_large" }, 413);
  const token = request.headers.get("x-sivtr-management-token");
  if (!token || !/^[A-Za-z0-9_-]{43}$/.test(token)) return json({ error: "not_found" }, 404);
  const body = new Uint8Array(await request.arrayBuffer());
  if (body.byteLength === 0 || body.byteLength > MAX_ENVELOPE_BYTES) return json({ error: "payload_too_large" }, 413);
  if (!validEnvelope(body)) return json({ error: "invalid_envelope" }, 400);
  const createdAt = new Date();
  const expiresAt = new Date(createdAt.getTime() + expiryMs(publication.expiry));
  const managementHash = await sha256(token);
  const existing = await env.PUBLICATIONS.head(publication.key);
  if (existing) return json({ error: "conflict" }, 409);
  const stored = await env.PUBLICATIONS.put(publication.key, body, {
    httpMetadata: { contentType: "application/octet-stream", cacheControl: "no-store" },
    customMetadata: {
      management_token_sha256: managementHash,
      created_at: createdAt.toISOString(),
      expires_at: expiresAt.toISOString(),
      envelope_version: "1",
    },
    // Conditional create prevents an id collision from overwriting a snapshot.
    onlyIf: { etagDoesNotMatch: "*" },
  });
  if (!stored) return json({ error: "conflict" }, 409);
  return new Response(null, { status: 201, headers: { "Cache-Control": "no-store" } });
}

async function get(request: Request, env: Env, publication: NonNullable<ReturnType<typeof parseId>>): Promise<Response> {
  if (await limited(env.GET_LIMITER, `${clientIp(request)}:${publication.id}`)) return json({ error: "rate_limited" }, 429);
  const object = await env.PUBLICATIONS.get(publication.key);
  if (!object || isExpired(object.customMetadata?.expires_at)) return json({ error: "not_found" }, 404);
  return new Response(object.body as unknown as BodyInit, {
    status: 200,
    headers: {
      "Content-Type": "application/octet-stream",
      "Cache-Control": "no-store, max-age=0",
      "Content-Length": String(object.size),
    },
  });
}

async function remove(request: Request, env: Env, publication: NonNullable<ReturnType<typeof parseId>>): Promise<Response> {
  if (await limited(env.REVOKE_LIMITER, `${clientIp(request)}:${publication.id}`)) return json({ error: "rate_limited" }, 429);
  const token = request.headers.get("x-sivtr-management-token");
  if (!token || !/^[A-Za-z0-9_-]{43}$/.test(token)) return json({ error: "not_found" }, 404);
  const object = await env.PUBLICATIONS.head(publication.key);
  if (!object || isExpired(object.customMetadata?.expires_at)) return json({ error: "not_found" }, 404);
  const expected = object.customMetadata?.management_token_sha256;
  const actual = await sha256(token);
  if (!expected || !timingSafeEqual(expected, actual)) return json({ error: "not_found" }, 404);
  await env.PUBLICATIONS.delete(publication.key);
  return new Response(null, { status: 204, headers: { "Cache-Control": "no-store" } });
}

function validEnvelope(body: Uint8Array): boolean {
  if (body.byteLength < 8 + 2 + 12 + 16 + 1) return false;
  for (let index = 0; index < MAGIC.length; index += 1) if (body[index] !== MAGIC[index]) return false;
  return body[8] === 1 && body[9] === 1;
}

function expiryMs(expiry: ExpiryClass): number {
  return { "1d": 86_400_000, "7d": 604_800_000, "30d": 2_592_000_000, "90d": 7_776_000_000 }[expiry];
}

function isExpired(value: string | undefined): boolean {
  if (!value) return true;
  const timestamp = Date.parse(value);
  return !Number.isFinite(timestamp) || timestamp <= Date.now();
}

async function limited(binding: RateLimit | undefined, key: string): Promise<boolean> {
  if (!binding) return false;
  return !(await binding.limit({ key })).success;
}

function clientIp(request: Request): string {
  return request.headers.get("cf-connecting-ip") ?? "unknown";
}

async function sha256(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function timingSafeEqual(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  return difference === 0;
}

function json(body: unknown, status: number, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", ...headers } });
}

function withSecurityHeaders(response: Response): Response {
  const headers = new Headers(response.headers);
  headers.set("X-Content-Type-Options", "nosniff");
  headers.set("Referrer-Policy", "no-referrer");
  headers.set("Permissions-Policy", "camera=(), microphone=(), geolocation=()");
  headers.set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'");
  if (response.headers.get("Content-Type")?.startsWith("text/html")) headers.set("X-Robots-Tag", "noindex, nofollow");
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

function errorCategory(error: unknown): string {
  return error instanceof Error ? error.name.slice(0, 32) : "unknown";
}

function logEvent(kind: string, request: Request, status: number, latencyMs: number, error?: string, size?: string | null): void {
  // Deliberately omit body, token, fragment key, complete publication id, and
  // decrypted metadata.  The route/method/status tuple is enough for ops.
  console.log(JSON.stringify({ kind, method: request.method, route: new URL(request.url).pathname.split("/").slice(0, 4).join("/"), status, latency_ms: latencyMs, size: size ? Number(size) : undefined, error }));
}

export { expiryMs, isExpired, parseId, validEnvelope };
