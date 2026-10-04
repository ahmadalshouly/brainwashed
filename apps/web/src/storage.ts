// What this browser remembers. Storage can be unavailable (private windows,
// blocked site data), so every access is guarded and the app works without it.

import type { ChatMessage, PairedHost } from "@brainwashed/api";

const HOST = "brainwashed.host";
const OLD_CHAT = "brainwashed.chat";
const CHATS = "brainwashed.chats";
const LOOK = "brainwashed.look";
const PAGE = "brainwashed.page";

function read<T>(key: string): T | null {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function write(key: string, value: unknown) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Not persisted; the session still works.
  }
}

export const loadHost = () => read<PairedHost>(HOST);
export const saveHost = (h: PairedHost | null) => write(HOST, h);

/** One conversation, kept only in this browser. */
export interface Conversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  updatedAt: number;
}

export const newId = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36);

export function loadChats(): Conversation[] {
  const chats = read<Conversation[]>(CHATS);
  if (chats) return chats;
  // The single conversation older versions kept.
  const old = read<ChatMessage[]>(OLD_CHAT);
  if (old?.length) {
    write(OLD_CHAT, null);
    return [{ id: newId(), title: titleFor(old), messages: old, updatedAt: Date.now() }];
  }
  return [];
}

/** Keeps the newest 100 conversations. */
export const saveChats = (c: Conversation[]) =>
  write(CHATS, c.length ? [...c].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 100) : null);

export function titleFor(messages: ChatMessage[]): string {
  const first = messages.find((m) => m.role === "user")?.content.trim() ?? "";
  const line = first.split("\n")[0];
  return line.length > 48 ? `${line.slice(0, 47)}…` : line || "New chat";
}

export type ThemeMode = "auto" | "light" | "dark";
export interface Look {
  theme: ThemeMode;
  accent: string;
}
export const ACCENTS = ["#4f46e5", "#0d9488", "#db2777", "#ea580c", "#2563eb", "#65a30d"];
export const loadLook = (): Look => ({ theme: "auto", accent: ACCENTS[0], ...read<Partial<Look>>(LOOK) });
export const saveLook = (l: Look) => write(LOOK, l);

export const loadPage = () => read<string>(PAGE);
export const savePage = (p: string) => write(PAGE, p);
