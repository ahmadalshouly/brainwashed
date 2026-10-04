// Turns files people pick, drop or paste into attachments. Pictures are
// shrunk in the browser so they travel fast and fit in storage; documents
// become text, read here when they are plain text and by the computer when
// they are PDFs or Word files.

import type { Attachment, RemoteHost } from "@brainwashed/api";

/** An attachment while it's being prepared. */
export interface Draft {
  id: string;
  name: string;
  kind: "image" | "file";
  status: "reading" | "ready" | "failed";
  /** Shown while composing: a picture's data URL. */
  preview?: string;
  detail?: string;
  error?: string;
  attachment?: Attachment;
}

/** Longest side of a picture sent to the model. Vision models scale down anyway. */
const MAX_SIDE = 1536;
const MAX_FILE_BYTES = 25 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;

const TEXT_TYPES = /^(text\/|application\/(json|xml|javascript|x-yaml|yaml|toml|x-sh|sql))/;
const TEXT_EXTENSIONS =
  /\.(txt|md|markdown|csv|tsv|json|jsonl|xml|html?|css|js|jsx|ts|tsx|py|rb|go|rs|java|kt|swift|c|h|cpp|hpp|cs|php|sh|bash|zsh|ps1|sql|ya?ml|toml|ini|cfg|conf|env|log|tex|srt|vtt)$/i;
const DOCUMENT_EXTENSIONS = /\.(pdf|docx)$/i;
const IMAGE_TYPES = /^image\/(png|jpeg|webp|gif|bmp|heic|heif|avif)$/;

export const ACCEPT = "image/*,.pdf,.docx,.txt,.md,.csv,.json,.xml,.html,.yaml,.yml,.log,text/*";

export function kindOf(file: File): "image" | "document" | "text" | null {
  if (IMAGE_TYPES.test(file.type)) return "image";
  if (DOCUMENT_EXTENSIONS.test(file.name) || file.type === "application/pdf") return "document";
  if (TEXT_TYPES.test(file.type) || TEXT_EXTENSIONS.test(file.name)) return "text";
  return null;
}

function toBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Draws the picture at most `MAX_SIDE` wide or tall, as JPEG (PNG if it has transparency and is small). */
async function shrink(file: File): Promise<{ mime: string; data: string }> {
  const bitmap = await createImageBitmap(file).catch(() => {
    throw new Error("This picture can't be opened in the browser. Try a PNG or JPEG.");
  });
  const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height));
  const w = Math.max(1, Math.round(bitmap.width * scale));
  const h = Math.max(1, Math.round(bitmap.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  // JPEG has no transparency; put pictures on white like most viewers do.
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, w, h);
  ctx.drawImage(bitmap, 0, 0, w, h);
  bitmap.close();
  const url = canvas.toDataURL("image/jpeg", 0.86);
  return { mime: "image/jpeg", data: url.slice(url.indexOf(",") + 1) };
}

export function dataUrl(a: Attachment): string | undefined {
  return a.type === "image" ? `data:${a.mime};base64,${a.data}` : undefined;
}

function size(n: number): string {
  return n < 1024 * 1024 ? `${Math.max(1, Math.round(n / 1024))} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Prepares one file. Resolves with the finished draft; never rejects. */
export async function prepare(file: File, remote: RemoteHost, id: string): Promise<Draft> {
  const base = { id, name: file.name || "Pasted picture" };
  const failed = (error: string): Draft => ({ ...base, kind: "file", status: "failed", error });
  const kind = kindOf(file);
  if (!kind) return failed("Attach pictures, PDFs, Word documents or text files.");
  if (file.size > MAX_FILE_BYTES) return failed(`Too big (${size(file.size)}). The limit is 25 MB.`);
  try {
    if (kind === "image") {
      const { mime, data } = await shrink(file);
      const attachment: Attachment = { type: "image", name: base.name, mime, data };
      return { ...base, kind: "image", status: "ready", preview: dataUrl(attachment), attachment };
    }
    if (kind === "text") {
      const text = await file.text();
      if (text.includes("\u0000")) return failed("This doesn't look like a text file.");
      return {
        ...base,
        kind: "file",
        status: "ready",
        detail: `${text.split("\n").length} lines`,
        attachment: { type: "file", name: base.name, text },
      };
    }
    const doc = await remote.readDocument(base.name, toBase64(await file.arrayBuffer()));
    const pages = doc.pages ? `${doc.pages} page${doc.pages === 1 ? "" : "s"}` : `${Math.round(doc.text.length / 1000)}k characters`;
    return {
      ...base,
      kind: "file",
      status: "ready",
      detail: doc.truncated ? `${pages}, shortened` : pages,
      attachment: { type: "file", name: base.name, text: doc.text },
    };
  } catch (e) {
    return failed(e instanceof Error ? e.message : String(e));
  }
}

export function extension(name: string): string {
  const m = /\.([a-z0-9]+)$/i.exec(name);
  return m ? m[1].toUpperCase().slice(0, 4) : "FILE";
}
