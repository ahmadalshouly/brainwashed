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
