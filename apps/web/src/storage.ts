// What this browser remembers. Storage can be unavailable (private windows,
// blocked site data), so every access is guarded and the app works without it.

import type { ChatMessage, PairedHost } from "@brainwashed/api";

const HOST = "brainwashed.host";
const CHAT = "brainwashed.chat";
const LOOK = "brainwashed.look";

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

export const loadChat = () => read<ChatMessage[]>(CHAT) ?? [];
export const saveChat = (m: ChatMessage[]) => write(CHAT, m.length ? m : null);

export type ThemeMode = "auto" | "light" | "dark";
export interface Look {
  theme: ThemeMode;
  accent: string;
}
export const ACCENTS = ["#4f46e5", "#0d9488", "#db2777", "#ea580c", "#2563eb", "#65a30d"];
export const loadLook = (): Look => ({ theme: "auto", accent: ACCENTS[0], ...read<Partial<Look>>(LOOK) });
export const saveLook = (l: Look) => write(LOOK, l);
