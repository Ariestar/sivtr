import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";

const fixture = JSON.parse(readFileSync(new URL("../tests/fixtures/rust-publication-v1.json", import.meta.url), "utf8")) as { publication_id: string; key: string; envelope_base64url: string };
const granularFixture = JSON.parse(readFileSync(new URL("../tests/fixtures/rust-publication-v2.json", import.meta.url), "utf8")) as { publication_id: string; key: string; envelope_base64url: string };
const xssFixture = JSON.parse(readFileSync(new URL("../tests/fixtures/xss-publication-v1.json", import.meta.url), "utf8")) as { publication_id: string; key: string; envelope_base64url: string };

test("missing fragment key is explicit and does not request a ciphertext", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  await page.goto("/s/7d_0123456789abcdefghijkl");
  await expect(page.locator(".error")).toContainText(/缺少链接密钥|missing its decryption key/);
  expect(requests.some((url) => url.includes("/api/v1/publications/"))).toBe(false);
});

test("decrypts the Rust-generated v1 fixture in the browser", async ({ page }) => {
  await page.route(`**/api/v1/publications/${fixture.publication_id}`, async (route) => {
    await route.fulfill({ status: 200, contentType: "application/octet-stream", body: Buffer.from(fixture.envelope_base64url, "base64url") });
  });
  await page.goto(`/s/${fixture.publication_id}#k=${fixture.key}`);
  await expect(page.locator("h1")).toHaveText("t");
  await expect(page.locator(".meta")).toContainText("codex");
});

test("renders granular v2 atoms with collapsed tools and gap markers", async ({ page }) => {
  await page.route(`**/api/v1/publications/${granularFixture.publication_id}`, async (route) => {
    await route.fulfill({ status: 200, contentType: "application/octet-stream", body: Buffer.from(granularFixture.envelope_base64url, "base64url") });
  });
  await page.goto(`/s/${granularFixture.publication_id}#k=${granularFixture.key}`);
  await expect(page.locator("h1")).toHaveText("Granular fixture");
  await expect(page.locator(".message.tool details")).toHaveCount(1);
  await expect(page.locator(".share-gap")).toContainText(/部分内容未分享|Some content was not shared/);
  await expect(page.locator(".message.tool details")).not.toHaveAttribute("open", "");
});

test("renders hostile Markdown as text without executing script", async ({ page }) => {
  await page.route(`**/api/v1/publications/${xssFixture.publication_id}`, async (route) => {
    await route.fulfill({ status: 200, contentType: "application/octet-stream", body: Buffer.from(xssFixture.envelope_base64url, "base64url") });
  });
  let dialogOpened = false;
  page.on("dialog", () => { dialogOpened = true; });
  await page.goto(`/s/${xssFixture.publication_id}#k=${xssFixture.key}`);
  await expect(page.locator("h1")).toHaveText("XSS fixture");
  await expect(page.locator(".markdown").first()).toContainText("<script>alert(1)</script>");
  expect(await page.locator(".markdown script").count()).toBe(0);
  expect(dialogOpened).toBe(false);
});
