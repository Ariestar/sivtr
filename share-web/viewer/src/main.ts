import DOMPurify from "dompurify";
import { gunzipSync } from "fflate";
import { marked } from "marked";
import "./style.css";

type Snapshot = {
  schema_version: number;
  title: string;
  provider: string;
  published_at: string;
  expires_at: string;
  items: Array<{ role: "user" | "assistant"; text: string; occurred_at: string | null }>;
};

const app = document.querySelector<HTMLElement>("#app")!;
const isEnglish = navigator.language.toLowerCase().startsWith("en");

void load();

async function load(): Promise<void> {
  const id = window.location.pathname.match(/^\/s\/([^/]+)$/)?.[1];
  const key = new URLSearchParams(window.location.hash.slice(1)).get("k");
  if (!key) return showError(t("缺少链接密钥", "The link is missing its decryption key."), false);
  if (!id || !validBase64Key(key)) return showError(t("链接密钥格式错误", "The link key is malformed."), false);
  try {
    const response = await fetch(`/api/v1/publications/${encodeURIComponent(id)}`, { cache: "no-store" });
    if (response.status === 404) return showError(t("链接已撤销、过期或不存在", "This link was revoked, expired, or does not exist."), false);
    if (!response.ok) throw new Error("network");
    const envelope = new Uint8Array(await response.arrayBuffer());
    const snapshot = await decryptEnvelope(envelope, key, id);
    if (snapshot.schema_version !== 1) return showError(t("不支持的快照版本", "This snapshot version is not supported."), false);
    render(snapshot);
  } catch (error) {
    if (error instanceof DOMException || (error instanceof Error && error.message === "decrypt")) {
      return showError(t("链接密钥错误或快照已损坏", "The key is wrong or the snapshot is corrupted."), false);
    }
    showError(t("网络暂时失败", "The network failed temporarily."), true);
  }
}

function validBase64Key(value: string): boolean {
  if (!/^[A-Za-z0-9_-]{43}$/.test(value)) return false;
  try { return base64url(value).byteLength === 32; } catch { return false; }
}

async function decryptEnvelope(envelope: Uint8Array, encodedKey: string, id: string): Promise<Snapshot> {
  try {
    if (envelope.byteLength < 39 || new TextDecoder().decode(envelope.slice(0, 8)) !== "SIVTPUB1" || envelope[8] !== 1 || envelope[9] !== 1) throw new Error("invalid envelope");
    const nonce = envelope.slice(10, 22);
    const ciphertext = envelope.slice(22);
    const key = await crypto.subtle.importKey("raw", base64url(encodedKey), "AES-GCM", false, ["decrypt"]);
    const plaintext = await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce, additionalData: new TextEncoder().encode(`sivtr-publication-v1:${id}`), tagLength: 128 }, key, ciphertext);
    const json = new TextDecoder().decode(gunzipSync(new Uint8Array(plaintext)));
    return JSON.parse(json) as Snapshot;
  } catch {
    throw new Error("decrypt");
  }
}

function base64url(value: string): Uint8Array {
  const binary = atob(value.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((value.length + 3) % 4));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

function render(snapshot: Snapshot): void {
  document.title = `${snapshot.title} · Sivtr`;
  app.replaceChildren();
  const header = document.createElement("header");
  const title = document.createElement("h1");
  title.textContent = snapshot.title;
  const meta = document.createElement("p");
  meta.className = "meta";
  meta.textContent = `${snapshot.provider} · ${formatDate(snapshot.published_at)} · ${t("只读快照", "read-only snapshot")}`;
  header.append(title, meta);
  app.append(header);
  const conversation = document.createElement("section");
  conversation.className = "conversation";
  for (const item of snapshot.items) {
    const article = document.createElement("article");
    article.className = `message ${item.role}`;
    const label = document.createElement("h2");
    label.textContent = item.role === "user" ? "User" : "Assistant";
    const body = document.createElement("div");
    body.className = "markdown";
    const escaped = item.text.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
    const html = marked.parse(escaped, { gfm: true, breaks: true, async: false }) as string;
    body.innerHTML = DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "script", "iframe", "object", "form", "base"] });
    body.querySelectorAll<HTMLAnchorElement>("a").forEach((anchor) => {
      anchor.rel = "noopener noreferrer";
      anchor.target = "_blank";
    });
    body.querySelectorAll<HTMLElement>("pre").forEach((pre) => {
      const wrapper = document.createElement("div");
      wrapper.className = "code-block";
      const copy = document.createElement("button");
      copy.type = "button";
      copy.className = "copy-code";
      copy.textContent = t("复制", "Copy");
      copy.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(pre.innerText);
          copy.textContent = t("已复制", "Copied");
          window.setTimeout(() => { copy.textContent = t("复制", "Copy"); }, 1200);
        } catch {
          copy.textContent = t("复制失败", "Copy failed");
        }
      });
      pre.replaceWith(wrapper);
      wrapper.append(pre, copy);
    });
    article.append(label, body);
    conversation.append(article);
  }
  app.append(conversation);
}

function showError(message: string, retry: boolean): void {
  app.replaceChildren();
  const panel = document.createElement("section");
  panel.className = "error";
  const heading = document.createElement("h1");
  heading.textContent = "Sivtr";
  const text = document.createElement("p");
  text.textContent = message;
  panel.append(heading, text);
  if (retry) {
    const button = document.createElement("button");
    button.textContent = t("重试", "Retry");
    button.addEventListener("click", () => void load());
    panel.append(button);
  }
  app.append(panel);
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat(isEnglish ? "en" : "zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function t(chinese: string, english: string): string { return isEnglish ? english : chinese; }
