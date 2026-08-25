import DOMPurify from "dompurify";
import { gunzipSync } from "fflate";
import { marked } from "marked";
import "./style.css";

type SnapshotBase = {
  schema_version: number;
  title: string;
  provider: string;
  published_at: string;
  expires_at: string;
};

type SnapshotV1 = SnapshotBase & {
  schema_version: 1;
  items: Array<{ role: "user" | "assistant"; text: string; occurred_at: string | null }>;
};

type SnapshotV2 = SnapshotBase & {
  schema_version: 2;
  items: Array<{
    kind: "user" | "assistant" | "tool" | "skill" | "thinking";
    label?: string;
    parts: Array<{
      kind: "user" | "assistant" | "tool_call" | "tool_result" | "skill" | "thinking";
      text: string;
      occurred_at: string | null;
    }>;
    gap_before?: boolean;
  }>;
};

type Snapshot = SnapshotV1 | SnapshotV2;

const app = document.querySelector<HTMLElement>("#app")!;
const isEnglish = navigator.language.toLowerCase().startsWith("en");

document.documentElement.lang = isEnglish ? "en" : "zh-CN";

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
    if (snapshot.schema_version !== 1 && snapshot.schema_version !== 2) return showError(t("不支持的快照版本", "This snapshot version is not supported."), false);
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
  document.title = `${snapshot.title} · sivtr`;
  const shell = el("div", "shell");
  shell.append(renderChrome(snapshot), renderConversation(snapshot), renderFooter(snapshot));
  app.replaceChildren(shell);
}

function renderChrome(snapshot?: Snapshot): HTMLElement {
  const header = el("header", "chrome");
  const brand = el("div", "brand");
  const logo = el("span", "logo");
  logo.setAttribute("aria-hidden", "true");
  brand.append(logo, el("span", "brand-name", "sivtr"), el("span", "chip", "readonly"));
  header.append(brand);
  if (!snapshot) return header;
  header.append(el("h1", undefined, snapshot.title));
  const meta = el("p", "meta");
  const provider = el("span", "provider", snapshot.provider);
  provider.dataset.name = snapshot.provider.toLowerCase();
  meta.append(
    provider,
    el("span", "sep", "·"),
    textSpan(formatDate(snapshot.published_at)),
    el("span", "sep", "·"),
    textSpan(t("只读快照", "read-only snapshot")),
  );
  header.append(meta);
  return header;
}

function renderConversation(snapshot: Snapshot): HTMLElement {
  const conversation = el("section", "conversation");
  if (snapshot.schema_version === 2) return renderGranularConversation(conversation, snapshot);
  for (const item of snapshot.items) {
    const article = el("article", `message ${item.role}`);
    const role = el("h2", "role");
    role.append(el("span", "dot"), document.createTextNode(item.role === "user" ? "User" : "Assistant"));
    const body = renderMarkdown(item.text);
    article.append(role, body);
    conversation.append(article);
  }
  return conversation;
}

function renderGranularConversation(conversation: HTMLElement, snapshot: SnapshotV2): HTMLElement {
  for (const item of snapshot.items) {
    if (item.gap_before) {
      conversation.append(el("div", "share-gap", t("部分内容未分享", "Some content was not shared")));
    }
    const article = el("article", `message atom ${item.kind}`);
    const role = el("h2", "role");
    role.append(el("span", "dot"), document.createTextNode(atomLabel(item.kind, item.label)));
    const content = el("div", "atom-content");
    for (const part of item.parts) {
      const body = renderMarkdown(part.text);
      if (item.parts.length > 1) {
        const partSection = el("section", "atom-part");
        partSection.append(el("h3", "part-kind", partLabel(part.kind)), body);
        content.append(partSection);
      } else {
        content.append(body);
      }
    }
    if (item.kind === "tool" || item.kind === "skill" || item.kind === "thinking") {
      const details = document.createElement("details");
      details.className = "atom-details";
      const summary = el("summary", undefined, t("展开查看", "Expand"));
      details.append(summary, content);
      article.append(role, details);
    } else {
      article.append(role, content);
    }
    conversation.append(article);
  }
  return conversation;
}

function renderMarkdown(text: string): HTMLElement {
  const body = el("div", "markdown");
  const escaped = text.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  const html = marked.parse(escaped, { gfm: true, breaks: true, async: false }) as string;
  body.innerHTML = DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "script", "iframe", "object", "form", "base"] });
  body.querySelectorAll<HTMLAnchorElement>("a").forEach((anchor) => {
    anchor.rel = "noopener noreferrer";
    anchor.target = "_blank";
  });
  body.querySelectorAll<HTMLElement>("pre").forEach((pre) => {
    const wrapper = el("div", "code-block");
    const copy = el("button", "copy-code", t("复制", "Copy"));
    copy.type = "button";
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
  return body;
}

function atomLabel(kind: SnapshotV2["items"][number]["kind"], label?: string): string {
  if (label) return label;
  return ({ user: "User", assistant: "Assistant", tool: "Tool", skill: "Skill", thinking: "Thinking" })[kind];
}

function partLabel(kind: SnapshotV2["items"][number]["parts"][number]["kind"]): string {
  return ({ user: "User", assistant: "Assistant", tool_call: "Tool call", tool_result: "Tool result", skill: "Skill", thinking: "Thinking" })[kind];
}

function renderFooter(snapshot: Snapshot): HTMLElement {
  const footer = el("footer", "page-footer");
  footer.textContent = [
    t(`有效至 ${formatDate(snapshot.expires_at)}`, `Expires ${formatDate(snapshot.expires_at)}`),
    t("端侧加密", "encrypted on device"),
    "sivtr",
  ].join("  ·  ");
  return footer;
}

function showError(message: string, retry: boolean): void {
  const shell = el("div", "shell");
  const chrome = renderChrome();
  const panel = el("section", "error error-panel");
  panel.append(el("h1", undefined, "Sivtr"), el("p", undefined, message));
  if (retry) {
    const button = el("button", undefined, t("重试", "Retry"));
    button.addEventListener("click", () => void load());
    panel.append(button);
  }
  shell.append(chrome, panel);
  app.replaceChildren(shell);
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat(isEnglish ? "en" : "zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(date);
}

function t(chinese: string, english: string): string { return isEnglish ? english : chinese; }

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function textSpan(text: string): HTMLSpanElement {
  return el("span", undefined, text);
}
