import { createHash, timingSafeEqual } from "node:crypto";
import { mkdir, open, readFile, readdir, stat, unlink } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_ENVELOPE_BYTES = 5 * 1024 * 1024;
const MAGIC = Buffer.from("SIVTPUB1", "ascii");
const ID_RE = /^(1d|7d|30d|90d)_([A-Za-z0-9_-]{22})$/;
const TOKEN_RE = /^[A-Za-z0-9_-]{43}$/;
const EXPIRY_MS = {
  "1d": 86_400_000,
  "7d": 604_800_000,
  "30d": 2_592_000_000,
  "90d": 7_776_000_000,
};
const SECURITY_HEADERS = {
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "no-referrer",
  "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
  "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
};
const MIME_TYPES = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
};

class HttpError extends Error {
  constructor(status, code) {
    super(code);
    this.name = "HttpError";
    this.status = status;
    this.code = code;
  }
}

export async function createPublicationServer(options = {}) {
  const moduleDir = fileURLToPath(new URL(".", import.meta.url));
  const dataDir = resolve(options.dataDir ?? process.env.DATA_DIR ?? "/var/lib/sivtr-share");
  const distDir = resolve(options.distDir ?? process.env.DIST_DIR ?? join(moduleDir, "..", "dist"));
  const now = options.now ?? (() => Date.now());
  const logger = options.logger ?? ((event) => console.log(JSON.stringify(event)));
  const createEnabled = options.createEnabled ?? process.env.CREATE_ENABLED !== "false";
  const limits = {
    create: options.limits?.create ?? 5,
    get: options.limits?.get ?? 120,
    revoke: options.limits?.revoke ?? 20,
    windowMs: options.limits?.windowMs ?? 60_000,
  };
  const rateBuckets = new Map();

  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  await cleanupExpired({ dataDir, now: now() });

  const server = createServer(async (request, response) => {
    const started = now();
    let status = 500;
    let size;
    let error;
    try {
      const result = await route(request);
      status = result.status;
      size = result.size;
      await send(response, result);
    } catch (cause) {
      const known = cause instanceof HttpError;
      status = known ? cause.status : 500;
      error = known ? cause.code : cause instanceof Error ? cause.name.slice(0, 32) : "unknown";
      await send(response, json(known ? error : "internal_error", status));
    } finally {
      logger({ method: request.method, route: routeLabel(request.url), status, latency_ms: Math.max(0, now() - started), size, error });
    }
  });

  const cleanupTimer = setInterval(() => {
    cleanupExpired({ dataDir, now: now() }).catch((cause) => {
      logger({ method: "SYSTEM", route: "cleanup", status: 500, error: cause instanceof Error ? cause.name.slice(0, 32) : "unknown" });
    });
  }, options.cleanupIntervalMs ?? 60 * 60 * 1000);
  cleanupTimer.unref();
  server.on("close", () => clearInterval(cleanupTimer));

  async function route(request) {
    const url = new URL(request.url ?? "/", "http://localhost");
    const apiMatch = url.pathname.match(/^\/api\/v1\/publications\/([^/]+)$/);
    if (apiMatch) return handleApi(request, apiMatch[1]);
    if (request.method !== "GET") throw new HttpError(404, "not_found");
    return serveAsset(url.pathname);
  }

  async function handleApi(request, id) {
    const publication = parseId(id, dataDir);
    if (!publication) throw new HttpError(404, "not_found");
    if (request.method === "PUT") return putPublication(request, publication);
    if (request.method === "GET") return getPublication(request, publication);
    if (request.method === "DELETE") return deletePublication(request, publication);
    return json("method_not_allowed", 405, { Allow: "GET, PUT, DELETE" });
  }

  async function putPublication(request, publication) {
    if (!createEnabled) throw new HttpError(503, "creation_disabled");
    enforceRateLimit(rateBuckets, `create:${clientIp(request)}`, limits.create, limits.windowMs, now());
    const declaredLength = Number(request.headers["content-length"] ?? "0");
    if (Number.isFinite(declaredLength) && declaredLength > MAX_ENVELOPE_BYTES) throw new HttpError(413, "payload_too_large");
    const token = header(request, "x-sivtr-management-token");
    if (!token || !TOKEN_RE.test(token)) throw new HttpError(404, "not_found");
    if (header(request, "content-type")?.split(";", 1)[0].trim().toLowerCase() !== "application/octet-stream") throw new HttpError(415, "unsupported_media_type");
    const body = await readRequestBody(request);
    if (!validEnvelope(body)) throw new HttpError(400, "invalid_envelope");

    const createdAt = new Date(now());
    const expiresAt = new Date(createdAt.getTime() + EXPIRY_MS[publication.expiry]);
    const metadata = {
      management_token_sha256: sha256(token),
      created_at: createdAt.toISOString(),
      expires_at: expiresAt.toISOString(),
      envelope_version: 1,
    };

    await mkdir(publication.directory, { recursive: true, mode: 0o700 });
    let envelopeHandle;
    try {
      envelopeHandle = await open(publication.envelopePath, "wx", 0o600);
      await envelopeHandle.writeFile(body);
      await envelopeHandle.sync();
    } catch (cause) {
      if (cause?.code === "EEXIST") throw new HttpError(409, "conflict");
      throw cause;
    } finally {
      await envelopeHandle?.close();
    }

    let metadataHandle;
    try {
      metadataHandle = await open(publication.metadataPath, "wx", 0o600);
      await metadataHandle.writeFile(`${JSON.stringify(metadata)}\n`, "utf8");
      await metadataHandle.sync();
    } catch (cause) {
      await safeUnlink(publication.envelopePath);
      if (cause?.code === "EEXIST") throw new HttpError(409, "conflict");
      throw cause;
    } finally {
      await metadataHandle?.close();
    }
    return empty(201);
  }

  async function getPublication(request, publication) {
    enforceRateLimit(rateBuckets, `get:${clientIp(request)}:${publication.random}`, limits.get, limits.windowMs, now());
    const metadata = await loadActiveMetadata(publication, now());
    if (!metadata) throw new HttpError(404, "not_found");
    let body;
    try {
      body = await readFile(publication.envelopePath);
    } catch (cause) {
      if (cause?.code === "ENOENT") throw new HttpError(404, "not_found");
      throw cause;
    }
    return { status: 200, body, size: body.byteLength, headers: { "Content-Type": "application/octet-stream", "Content-Length": String(body.byteLength) } };
  }

  async function deletePublication(request, publication) {
    enforceRateLimit(rateBuckets, `revoke:${clientIp(request)}`, limits.revoke, limits.windowMs, now());
    const token = header(request, "x-sivtr-management-token");
    if (!token || !TOKEN_RE.test(token)) throw new HttpError(404, "not_found");
    const metadata = await loadActiveMetadata(publication, now());
    if (!metadata || !safeHashEqual(metadata.management_token_sha256, sha256(token))) throw new HttpError(404, "not_found");
    await Promise.all([safeUnlink(publication.envelopePath), safeUnlink(publication.metadataPath)]);
    return empty(204);
  }

  async function serveAsset(pathname) {
    const requested = pathname === "/" || pathname.startsWith("/s/") ? "/index.html" : pathname;
    const safePath = normalize(requested).replace(/^(\.\.[/\\])+/, "").replace(/^[/\\]+/, "");
    const assetPath = resolve(distDir, safePath);
    if (assetPath !== distDir && !assetPath.startsWith(`${distDir}\\`) && !assetPath.startsWith(`${distDir}/`)) throw new HttpError(404, "not_found");
    let info;
    try {
      info = await stat(assetPath);
    } catch (cause) {
      if (cause?.code === "ENOENT") throw new HttpError(404, "not_found");
      throw cause;
    }
    if (!info.isFile()) throw new HttpError(404, "not_found");
    const type = MIME_TYPES[extname(assetPath).toLowerCase()] ?? "application/octet-stream";
    const body = await readFile(assetPath);
    return { status: 200, body, size: body.byteLength, headers: { "Content-Type": type, "Content-Length": String(body.byteLength), ...(type.startsWith("text/html") ? { "X-Robots-Tag": "noindex, nofollow" } : {}) } };
  }

  return server;
}

export async function cleanupExpired({ dataDir, now = Date.now() }) {
  const root = join(resolve(dataDir), "v1");
  for (const expiry of Object.keys(EXPIRY_MS)) {
    const directory = join(root, expiry);
    let entries;
    try {
      entries = await readdir(directory, { withFileTypes: true });
    } catch (cause) {
      if (cause?.code === "ENOENT") continue;
      throw cause;
    }
    const names = new Set(entries.filter((entry) => entry.isFile()).map((entry) => entry.name));
    for (const name of names) {
      if (!name.endsWith(".json")) continue;
      const random = name.slice(0, -5);
      if (!/^[A-Za-z0-9_-]{22}$/.test(random)) continue;
      const metadataPath = join(directory, name);
      const envelopePath = join(directory, `${random}.bin`);
      const metadata = await readMetadata(metadataPath);
      if (!metadata || isExpired(metadata.expires_at, now) || !names.has(`${random}.bin`)) {
        await Promise.all([safeUnlink(metadataPath), safeUnlink(envelopePath)]);
      }
    }
    for (const name of names) {
      if (!name.endsWith(".bin") || names.has(`${name.slice(0, -4)}.json`)) continue;
      const path = join(directory, name);
      const info = await stat(path);
      if (info.mtimeMs <= now - 5 * 60_000) await safeUnlink(path);
    }
  }
}

function parseId(id, dataDir) {
  const match = ID_RE.exec(id);
  if (!match) return null;
  const directory = join(dataDir, "v1", match[1]);
  return { id, expiry: match[1], random: match[2], directory, envelopePath: join(directory, `${match[2]}.bin`), metadataPath: join(directory, `${match[2]}.json`) };
}

async function loadActiveMetadata(publication, now) {
  const metadata = await readMetadata(publication.metadataPath);
  if (!metadata || isExpired(metadata.expires_at, now)) {
    if (metadata) await Promise.all([safeUnlink(publication.envelopePath), safeUnlink(publication.metadataPath)]);
    return null;
  }
  return metadata;
}

async function readMetadata(path) {
  try {
    const value = JSON.parse(await readFile(path, "utf8"));
    if (typeof value !== "object" || value === null) return null;
    return value;
  } catch (cause) {
    if (cause?.code === "ENOENT" || cause instanceof SyntaxError) return null;
    throw cause;
  }
}

function validEnvelope(body) {
  return body.byteLength >= 8 + 2 + 12 + 16 + 1 && body.subarray(0, 8).equals(MAGIC) && body[8] === 1 && body[9] === 1;
}

async function readRequestBody(request) {
  const chunks = [];
  let total = 0;
  for await (const chunk of request) {
    total += chunk.byteLength;
    if (total > MAX_ENVELOPE_BYTES) throw new HttpError(413, "payload_too_large");
    chunks.push(chunk);
  }
  if (total === 0) throw new HttpError(413, "payload_too_large");
  return Buffer.concat(chunks, total);
}

function enforceRateLimit(buckets, key, limit, windowMs, timestamp) {
  const current = buckets.get(key);
  if (!current || current.resetAt <= timestamp) {
    if (buckets.size >= 10_000) {
      for (const [bucketKey, bucket] of buckets) if (bucket.resetAt <= timestamp) buckets.delete(bucketKey);
      if (buckets.size >= 10_000 && !buckets.has(key)) throw new HttpError(429, "rate_limited");
    }
    buckets.set(key, { count: 1, resetAt: timestamp + windowMs });
    return;
  }
  current.count += 1;
  if (current.count > limit) throw new HttpError(429, "rate_limited");
}

function clientIp(request) {
  return header(request, "x-real-ip") ?? header(request, "x-forwarded-for")?.split(",")[0].trim() ?? request.socket.remoteAddress ?? "unknown";
}

function header(request, name) {
  const value = request.headers[name];
  return Array.isArray(value) ? value[0] : value;
}

function sha256(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

function safeHashEqual(left, right) {
  if (typeof left !== "string" || left.length !== right.length) return false;
  return timingSafeEqual(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"));
}

function isExpired(value, now) {
  const timestamp = Date.parse(value);
  return !Number.isFinite(timestamp) || timestamp <= now;
}

function json(code, status, headers = {}) {
  const body = Buffer.from(JSON.stringify({ error: code }));
  return { status, body, size: body.byteLength, headers: { "Content-Type": "application/json; charset=utf-8", "Content-Length": String(body.byteLength), ...headers } };
}

function empty(status) {
  return { status, body: null, size: 0, headers: {} };
}

async function send(response, result) {
  response.writeHead(result.status, { "Cache-Control": "no-store, max-age=0", ...SECURITY_HEADERS, ...result.headers });
  response.end(result.body);
}

function routeLabel(rawUrl) {
  const pathname = new URL(rawUrl ?? "/", "http://localhost").pathname;
  if (/^\/api\/v1\/publications\//.test(pathname)) return "/api/v1/publications/:id";
  if (pathname.startsWith("/s/")) return "/s/:id";
  if (pathname.startsWith("/assets/")) return "/assets/:file";
  return pathname.slice(0, 64);
}

async function safeUnlink(path) {
  try {
    await unlink(path);
  } catch (cause) {
    if (cause?.code !== "ENOENT") throw cause;
  }
}

async function main() {
  const host = process.env.HOST ?? "127.0.0.1";
  const port = Number(process.env.PORT ?? "8791");
  const server = await createPublicationServer();
  server.listen(port, host, () => console.log(JSON.stringify({ kind: "startup", host, port })));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((cause) => {
    console.error(JSON.stringify({ kind: "startup_error", error: cause instanceof Error ? cause.name : "unknown" }));
    process.exitCode = 1;
  });
}

export { MAX_ENVELOPE_BYTES, parseId, validEnvelope };
