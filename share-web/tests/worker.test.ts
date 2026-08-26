import { describe, expect, it } from "vitest";
import worker, { expiryMs, isExpired, parseId, readCappedBody, validEnvelope } from "../src/worker";

describe("publication route primitives", () => {
  it("parses expiry-class ids without accepting arbitrary paths", () => {
    const parsed = parseId("7d_0123456789abcdefghijkl");
    expect(parsed?.key).toBe("v1/7d/0123456789abcdefghijkl");
    expect(parseId("7d_bad")).toBeNull();
  });

  it("uses the locked expiry classes", () => {
    expect(expiryMs("2h")).toBe(7_200_000);
    expect(expiryMs("1d")).toBe(86_400_000);
    expect(expiryMs("3d")).toBe(259_200_000);
    expect(expiryMs("90d")).toBe(7_776_000_000);
    expect(isExpired("2000-01-01T00:00:00.000Z")).toBe(true);
  });

  it("does not confuse 3d with 30d prefixes", () => {
    const token = "0123456789abcdefghijkl";
    expect(parseId(`3d_${token}`)?.expiry).toBe("3d");
    expect(parseId(`30d_${token}`)?.expiry).toBe("30d");
    expect(parseId(`2h_${token}`)?.key).toBe(`v1/2h/${token}`);
  });

  it("only accepts the v1 gzip envelope header", () => {
    const envelope = new Uint8Array(39);
    envelope.set(new TextEncoder().encode("SIVTPUB1"));
    envelope[8] = 1;
    envelope[9] = 1;
    expect(validEnvelope(envelope)).toBe(true);
    envelope[9] = 0;
    expect(validEnvelope(envelope)).toBe(false);
  });

  it("keeps a recent client publication timestamp as the shared expiry authority", async () => {
    const objects = new Map<string, { bytes: Uint8Array; customMetadata: Record<string, string> }>();
    const bucket = {
      async head(key: string) {
        return objects.get(key) ?? null;
      },
      async put(key: string, body: Uint8Array, options: { customMetadata: Record<string, string>; onlyIf?: unknown }) {
        objects.set(key, { bytes: new Uint8Array(body), customMetadata: options.customMetadata });
        return {};
      },
    };
    const env = { PUBLICATIONS: bucket, CREATE_ENABLED: "true" } as any;
    const id = "7d_0123456789abcdefghijkx";
    const publishedAt = new Date(Date.now() - 1_000).toISOString();
    const envelope = new Uint8Array(39);
    envelope.set(new TextEncoder().encode("SIVTPUB1"));
    envelope[8] = 1;
    envelope[9] = 1;

    const response = await worker.fetch(
      new Request(`https://share.sivtr.dev/api/v1/publications/${id}`, {
        method: "PUT",
        headers: {
          "x-sivtr-management-token": "A".repeat(43),
          "x-sivtr-published-at": publishedAt,
          "content-type": "application/octet-stream",
        },
        body: envelope as unknown as BodyInit,
      }),
      env,
      {} as any,
    );

    expect(response.status).toBe(201);
    expect(objects.get("v1/7d/0123456789abcdefghijkx")?.customMetadata.created_at).toBe(publishedAt);
  });

  it("keeps PUT/GET/DELETE opaque and makes revoke immediately unreadable", async () => {
    const objects = new Map<string, { bytes: Uint8Array; customMetadata: Record<string, string> }>();
    const bucket = {
      async head(key: string) {
        const object = objects.get(key);
        return object ? { size: object.bytes.byteLength, customMetadata: object.customMetadata } : null;
      },
      async put(key: string, body: Uint8Array, options: { customMetadata: Record<string, string>; onlyIf?: unknown }) {
        if (objects.has(key)) return null;
        objects.set(key, { bytes: new Uint8Array(body), customMetadata: options.customMetadata });
        return {};
      },
      async get(key: string) {
        const object = objects.get(key);
        if (!object) return null;
        return { body: new Response(object.bytes as unknown as BodyInit).body, size: object.bytes.byteLength, customMetadata: object.customMetadata };
      },
      async delete(key: string) { objects.delete(key); },
    };
    const env = { PUBLICATIONS: bucket, ASSETS: { fetch: async () => new Response("shell", { headers: { "Content-Type": "text/html" } }) }, CREATE_ENABLED: "true" } as any;
    const id = "7d_0123456789abcdefghijkl";
    const token = "A".repeat(43);
    const envelope = new Uint8Array(39);
    envelope.set(new TextEncoder().encode("SIVTPUB1"));
    envelope[8] = 1;
    envelope[9] = 1;
    const put = await worker.fetch(new Request(`https://share.sivtr.dev/api/v1/publications/${id}`, { method: "PUT", headers: { "x-sivtr-management-token": token, "content-type": "application/octet-stream" }, body: envelope as unknown as BodyInit }), env, {} as any);
    expect(put.status).toBe(201);
    const get = await worker.fetch(new Request(`https://share.sivtr.dev/api/v1/publications/${id}`), env, {} as any);
    expect(get.status).toBe(200);
    const wrongDelete = await worker.fetch(new Request(`https://share.sivtr.dev/api/v1/publications/${id}`, { method: "DELETE", headers: { "x-sivtr-management-token": "B".repeat(43) } }), env, {} as any);
    expect(wrongDelete.status).toBe(404);
    const remove = await worker.fetch(new Request(`https://share.sivtr.dev/api/v1/publications/${id}`, { method: "DELETE", headers: { "x-sivtr-management-token": token } }), env, {} as any);
    expect(remove.status).toBe(204);
    const after = await worker.fetch(new Request(`https://share.sivtr.dev/api/v1/publications/${id}`), env, {} as any);
    expect(after.status).toBe(404);
  });

  it("stops reading a PUT body once it exceeds 5 MiB", async () => {
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        const chunk = new Uint8Array(64 * 1024);
        for (let sent = 0; sent <= 5 * 1024 * 1024; sent += chunk.byteLength) {
          controller.enqueue(chunk);
        }
        controller.close();
      },
    });
    const body = await readCappedBody(stream, 5 * 1024 * 1024);
    expect(body).toBeNull();
  });
});
