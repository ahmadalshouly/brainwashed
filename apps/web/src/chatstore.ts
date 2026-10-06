// Conversations live in this browser's IndexedDB, which has room for the
// pictures people attach. Older versions kept them in localStorage; they move
// over on first load. When IndexedDB is unavailable (some private windows)
// chats last until the page closes.

import type { Attachment, ChatMessage, ChatOptions, ReplyStats, ToolCall, ToolResult } from "@brainwashed/api";
import { loadChats as loadLegacyChats, saveChats as clearLegacyChats } from "./storage";

/** A message as the chat shows it, with what streamed alongside the answer. */
export interface Turn extends ChatMessage {
  attachments?: Attachment[];
  skills?: string[];
  reasoning?: string;
  /** How long the model thought, in seconds. */
  thoughtFor?: number;
  stats?: ReplyStats;
  /** Tools the model called. */
  toolCalls?: ShownCall[];
  error?: string;
}

/** A tool call as the chat shows it. */
export interface ShownCall extends ToolCall {
  /** What the tool returned, for calls the host ran. */
  result?: ToolResult;
  /** Where in the answer's text the call was made. */
  at?: number;
}

export interface Conversation {
  id: string;
  title: string;
  turns: Turn[];
  updatedAt: number;
  pinned?: boolean;
  /** Extra instructions for this conversation only. */
  instructions?: string;
}

const DB = "brainwashed";
const STORE = "chats";
/** Oldest conversations past this are dropped, pinned ones last. */
const MAX_CHATS = 300;

let dbPromise: Promise<IDBDatabase | null> | null = null;

function db(): Promise<IDBDatabase | null> {
  dbPromise ??= new Promise((resolve) => {
    try {
      const req = indexedDB.open(DB, 1);
      req.onupgradeneeded = () => req.result.createObjectStore(STORE, { keyPath: "id" });
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
  return dbPromise;
}

function run<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T> | void): Promise<T | undefined> {
  return db().then(
    (d) =>
      new Promise((resolve) => {
        if (!d) return resolve(undefined);
        try {
          const tx = d.transaction(STORE, mode);
          const req = f(tx.objectStore(STORE));
          tx.oncomplete = () => resolve(req ? req.result : undefined);
          tx.onerror = () => resolve(undefined);
          tx.onabort = () => resolve(undefined);
        } catch {
          resolve(undefined);
        }
      }),
  );
}

export async function loadConversations(): Promise<Conversation[]> {
  let all = (await run<Conversation[]>("readonly", (s) => s.getAll() as IDBRequest<Conversation[]>)) ?? [];
  const legacy = loadLegacyChats();
  if (legacy.length) {
    const moved: Conversation[] = legacy.map((c) => ({
      id: c.id,
      title: c.title,
      turns: c.messages,
      updatedAt: c.updatedAt,
    }));
    await Promise.all(moved.map(saveConversation));
    clearLegacyChats([]);
    all = [...moved, ...all.filter((c) => !moved.some((m) => m.id === c.id))];
  }
  return sortChats(all);
}

export function sortChats(chats: Conversation[]): Conversation[] {
  return [...chats].sort((a, b) => Number(!!b.pinned) - Number(!!a.pinned) || b.updatedAt - a.updatedAt);
}

export async function saveConversation(c: Conversation): Promise<void> {
  // Errors are shown, not kept.
  const turns = c.turns.filter((t) => !t.error || t.content).map(({ error: _, ...t }) => t);
  await run("readwrite", (s) => s.put({ ...c, turns }));
}

export async function deleteConversation(id: string): Promise<void> {
  await run("readwrite", (s) => s.delete(id));
}

export async function clearConversations(): Promise<void> {
  await run("readwrite", (s) => s.clear());
}

/** Drops the oldest conversations so storage doesn't grow forever. */
export async function pruneConversations(chats: Conversation[]): Promise<Conversation[]> {
  if (chats.length <= MAX_CHATS) return chats;
  const keep = sortChats(chats).slice(0, MAX_CHATS);
  await Promise.all(chats.filter((c) => !keep.includes(c)).map((c) => deleteConversation(c.id)));
  return keep;
}

export const newId = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36);

export function titleFor(turns: ChatMessage[]): string {
  const first = turns.find((m) => m.role === "user");
  const text = first?.content.trim() || first?.attachments?.[0]?.name || "";
  const line = text.split("\n")[0];
  return line.length > 52 ? `${line.slice(0, 51)}…` : line || "New chat";
}

/** The conversation as Markdown, for saving or sharing. */
export function toMarkdown(c: Conversation): string {
  const parts = [`# ${c.title}`, ""];
  for (const t of c.turns) {
    parts.push(t.role === "user" ? "## You" : "## BrainWashed", "");
    for (const a of t.attachments ?? []) parts.push(`*Attached: ${a.name}*`, "");
    parts.push(t.content, "");
  }
  return parts.join("\n");
}

// ----- model settings, kept per browser -----

const OPTIONS = "brainwashed.chatOptions";

export interface ChatPrefs extends ChatOptions {
  /** Named temperature preset, or "custom". */
  style?: "precise" | "balanced" | "creative" | "custom";
}

export const STYLES = {
  precise: { label: "Precise", temperature: 0.2, hint: "Focused and consistent. Good for facts and code." },
  balanced: { label: "Balanced", temperature: 0.7, hint: "The everyday default." },
  creative: { label: "Creative", temperature: 1.1, hint: "More varied. Good for ideas and writing." },
} as const;

export function loadPrefs(): ChatPrefs {
  try {
    return JSON.parse(localStorage.getItem(OPTIONS) ?? "{}") as ChatPrefs;
  } catch {
    return {};
  }
}

export function savePrefs(p: ChatPrefs) {
  try {
    localStorage.setItem(OPTIONS, JSON.stringify(p));
  } catch {
    // Not kept; still used for this visit.
  }
}

/** What goes over the wire: the options without UI-only fields. */
export function toOptions(p: ChatPrefs): ChatOptions | undefined {
  const { style: _, ...options } = p;
  const set = Object.fromEntries(Object.entries(options).filter(([, v]) => v !== undefined && v !== null && !Number.isNaN(v)));
  return Object.keys(set).length ? (set as ChatOptions) : undefined;
}
