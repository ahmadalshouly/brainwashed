/** Shared wire types between the host gateway, the host UI and the mobile app. */

export type Role = "system" | "user" | "assistant";

export interface ChatMessage {
  role: Role;
  content: string;
}

export interface HostInfo {
  /** Human-readable name of the host, e.g. "Ahmad's MacBook". */
  name: string;
  /** Semver of the BrainWashed host app. */
  version: string;
  /** Id of the model currently loaded, if any. */
  model: string | null;
}

export interface SkillSummary {
  name: string;
  description: string;
  enabled: boolean;
}

export interface ChatRequest {
  messages: ChatMessage[];
  /** Model id; the host's active model is used when omitted. */
  model?: string;
}

export interface ChatResponse {
  message: ChatMessage;
}

/** A piece of a streamed answer. Mirrors `Delta` in crates/runtime. */
export type Delta = { kind: "content"; text: string } | { kind: "reasoning"; text: string };

/** Mirrors `EngineState` in crates/core. */
export type EngineState =
  | { state: "idle" }
  | { state: "installingRuntime"; done: number; total: number | null }
  | { state: "loading"; model: string }
  | { state: "ready"; model: string }
  | { state: "error"; message: string };

export interface InstalledModel {
  id: string;
  name: string;
  repo: string | null;
  path: string;
  size: number;
}

export interface CatalogItem {
  repo: string;
  name: string;
  description: string;
  params_b: number;
  license: string;
  installed: boolean;
  fits: boolean;
}

/** Mirrors `Event` in crates/core. */
export type EngineEvent =
  | ({ type: "state" } & EngineState)
  | { type: "downloadProgress"; repo: string; done: number; total: number | null }
  | { type: "downloadFinished"; repo: string; model: InstalledModel }
  | { type: "downloadFailed"; repo: string; error: string }
  | { type: "modelsChanged" };
