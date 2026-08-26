import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createPublicationServer } from "./self-host.mjs";

const TOKEN = "A".repeat(43);
const WRONG_TOKEN = "B".repeat(43);
const ID = `7d_${"c".repeat(22)}`;

const running = [];

afterEach(async () => {
  await Promise.all(running.splice(0).map(({ server, root }) => new Promise((resolve) => server.close(() => resolve())).then(() => rm(root, { recursive: true, force: true }))));
});

describe("self-hosted publication service", () => {
  it("creates, reads, and revokes an immutable encrypted envelope", async () => {
    const fixture = await start();
    const envelope = validEnvelope("private conversation bytes");

    expect((await put(fixture.base, ID, TOKEN, envelope)).status).toBe(201);
    expect((await put(fixture.base, ID, TOKEN, envelope)).status).toBe(409);

    const stored = await fetch(`${fixture.base}/api/v1/publications/${ID}`);
    expect(stored.status).toBe(200);
    expect(Buffer.from(await stored.arrayBuffer())).toEqual(envelope);
    expect(stored.headers.get("cache-control")).toContain("no-store");

    const metadataText = await readFile(join(fixture.dataDir, "v1", "7d", `${"c".repeat(22)}.json`), "utf8");
    const metadata = JSON.parse(metadataText);
    expect(Object.keys(metadata).sort()).toEqual(["created_at", "envelope_version", "expires_at", "management_token_sha256"]);
    expect(metadataText).not.toContain("private conversation bytes");
    expect(metadataText).not.toContain(TOKEN);

    expect((await remove(fixture.base, ID, WRONG_TOKEN)).status).toBe(404);
    expect((await remove(fixture.base, ID, TOKEN)).status).toBe(204);
    expect((await fetch(`${fixture.base}/api/v1/publications/${ID}`)).status).toBe(404);
  });

  it("enforces exact expiry and deletes expired local files", async () => {
    let clock = Date.parse("2026-08-25T00:00:00Z");
    const fixture = await start({ now: () => clock });
    const id = `1d_${"d".repeat(22)}`;
    expect((await put(fixture.base, id, TOKEN, validEnvelope())).status).toBe(201);
    clock += 86_400_000;
    expect((await fetch(`${fixture.base}/api/v1/publications/${id}`)).status).toBe(404);
    await expect(readFile(join(fixture.dataDir, "v1", "1d", `${"d".repeat(22)}.json`))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("serves the fixed viewer shell with restrictive security headers", async () => {
    const fixture = await start();
    const response = await fetch(`${fixture.base}/s/${ID}#k=never-sent`);
    expect(response.status).toBe(200);
    expect(await response.text()).toContain("Sivtr test viewer");
    expect(response.headers.get("content-security-policy")).toContain("frame-ancestors 'none'");
    expect(response.headers.get("x-robots-tag")).toBe("noindex, nofollow");
  });

  it("validates media type and applies the configured per-IP create limit", async () => {
    const fixture = await start({ limits: { create: 1, get: 120, revoke: 20, windowMs: 60_000 } });
    const invalidType = await fetch(`${fixture.base}/api/v1/publications/${ID}`, {
      method: "PUT",
      headers: { "x-sivtr-management-token": TOKEN, "content-type": "text/plain" },
      body: validEnvelope(),
    });
    expect(invalidType.status).toBe(415);

    // Rate limiting happens before body processing, so the next create from
    // the same address is rejected without creating any file.
    const secondId = `7d_${"e".repeat(22)}`;
    expect((await put(fixture.base, secondId, TOKEN, validEnvelope())).status).toBe(429);
  });
});

async function start(overrides = {}) {
  const root = await mkdtemp(join(tmpdir(), "sivtr-share-test-"));
  const dataDir = join(root, "data");
  const distDir = join(root, "dist");
  await mkdir(distDir, { recursive: true });
  await writeFile(join(distDir, "index.html"), "<!doctype html><title>Sivtr test viewer</title>");
  const server = await createPublicationServer({ dataDir, distDir, logger: () => {}, ...overrides });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const fixture = { server, root, dataDir, base: `http://127.0.0.1:${address.port}` };
  running.push(fixture);
  return fixture;
}

function validEnvelope(suffix = "x") {
  return Buffer.concat([Buffer.from("SIVTPUB1", "ascii"), Buffer.from([1, 1]), Buffer.alloc(12), Buffer.from(suffix), Buffer.alloc(16)]);
}

function put(base, id, token, body) {
  return fetch(`${base}/api/v1/publications/${id}`, {
    method: "PUT",
    headers: { "x-sivtr-management-token": token, "content-type": "application/octet-stream" },
    body,
  });
}

function remove(base, id, token) {
  return fetch(`${base}/api/v1/publications/${id}`, { method: "DELETE", headers: { "x-sivtr-management-token": token } });
}
