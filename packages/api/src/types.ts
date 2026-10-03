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

/** A skill as the host lists it. Mirrors `SkillInfo` in crates/core. */
export interface SkillInfo extends SkillSummary {
  triggers: string[];
  version: number;
  path: string;
}

export interface SkillList {
  /** Folder the skills live in, one subfolder per skill. */
  dir: string;
  skills: SkillInfo[];
  /** Skill files that could not be loaded. */
  errors: { path: string; message: string }[];
}

export interface ChatRequest {
  messages: ChatMessage[];
  /** Model id; the host's active model is used when omitted. */
  model?: string;
}

export interface ChatResponse {
  message: ChatMessage;
}

/** What streams back during a reply. Mirrors `ChatEvent` in crates/core. */
export type ChatEvent =
  | { kind: "skills"; names: string[] }
  | { kind: "content"; text: string }
  | { kind: "reasoning"; text: string };

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
  | { type: "modelsChanged" }
  | { type: "skillsChanged" };
