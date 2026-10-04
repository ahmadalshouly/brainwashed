/** Shared wire types between the host gateway, the host UI and the mobile app. */

export type Role = "system" | "user" | "assistant";

export interface ChatMessage {
  role: Role;
  content: string;
  /** Pictures and documents sent with the message. */
  attachments?: Attachment[];
}

/**
 * Something sent with a message. Mirrors `Attachment` in crates/runtime.
 * Pictures reach the model only if it has a vision projector (see
 * `InstalledModel.mmproj`); others read "[Picture: name]". Documents travel as
 * text: read PDFs and Word files with `readDocument` first.
 */
export type Attachment =
  | { type: "image"; name: string; /** image/png, image/jpeg, image/webp or image/gif */ mime: string; /** base64 */ data: string }
  | { type: "file"; name: string; text: string };

/** A model the chat can use. Mirrors `ChatModel` in crates/core. */
export interface ChatModel {
  /** "local", or "<provider id>/<model>". */
  id: string;
  name: string;
  /** Provider name; null for the model running on the computer. */
  provider: string | null;
  vision: boolean;
  /** Messages leave the computer. */
  cloud: boolean;
}

/** A cloud provider as admins see it. The API key never leaves the computer. */
export interface ProviderInfo {
  id: string;
  name: string;
  baseUrl: string;
  models: string[];
  /** Members may use these models too. */
  members: boolean;
  keySet: boolean;
  /** Last characters of the key, e.g. "…a1b2". */
  keyHint: string | null;
}

export interface ProviderInput {
  id: string;
  name: string;
  baseUrl: string;
  /** Left out keeps the saved key; "" removes it. */
  apiKey?: string;
  models: string[];
  members: boolean;
}

/** What `readDocument` returns. */
export interface DocumentText {
  text: string;
  /** Pages, for PDFs. */
  pages?: number;
  /** The text was cut to fit. */
  truncated: boolean;
}

/**
 * How the model picks its words, sent with each chat. Anything left out uses
 * the model's defaults. Mirrors `SamplingOptions` in crates/runtime.
 */
export interface ChatOptions {
  /** 0 to 2. Lower is more focused, higher more creative. */
  temperature?: number;
  /** 0 to 1. */
  topP?: number;
  /** Up to 1000. */
  topK?: number;
  /** 0 to 1. */
  minP?: number;
  /** 0.5 to 2. */
  repeatPenalty?: number;
  /** -2 to 2. */
  presencePenalty?: number;
  seed?: number;
  /** Longest reply, in tokens. */
  maxTokens?: number;
  /** Whether reasoning models think first. Left out keeps the model's default. */
  reasoning?: boolean;
}

/** How a reply went. Sent once, at the end. */
export interface ReplyStats {
  promptTokens: number;
  tokens: number;
  tokensPerSecond: number;
  /** The reply hit `maxTokens` before it finished. */
  truncated: boolean;
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
  /** Ships with BrainWashed: it can be turned off but not deleted. */
  builtin?: boolean;
  /** Where it was installed from; missing for skills written on the host. */
  origin?: SkillOrigin | null;
  /** Changed on the host since it was installed. */
  modified?: boolean;
}

/** Where an installed skill came from. Mirrors `SkillOrigin` in crates/core. */
export interface SkillOrigin {
  url: string;
  sha256: string;
  /** Installed from the community index rather than a link. */
  community: boolean;
  /** Seconds since 1970. */
  installedAt: number;
}

/** A skill in the community index. Mirrors `CommunitySkill` in crates/core. */
export interface CommunitySkill {
  name: string;
  description: string;
  triggers: string[];
  version: number;
  author?: string | null;
  category?: string | null;
  /** The SKILL.md, pinned to the commit it was reviewed at. */
  url: string;
  sha256: string;
  page?: string | null;
}

/** A skill downloaded for reading before it's installed. */
export interface SkillPreview {
  name: string;
  description: string;
  source: string;
  url: string;
  sha256: string;
  community: boolean;
  /** Lines worth a close look, e.g. ones that try to override instructions. */
  warnings: string[];
  /** A skill with this name is already installed. */
  installed: boolean;
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
  | { kind: "reasoning"; text: string }
  | ({ kind: "stats" } & ReplyStats);

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
  /** Vision projector. Set when the model can look at pictures. */
  mmproj?: string;
}

export interface CatalogItem {
  repo: string;
  name: string;
  description: string;
  params_b: number;
  license: string;
  /** Can look at pictures. */
  vision?: boolean;
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

/** Admins manage the host; members chat. Mirrors `DeviceRole` in crates/gateway. */
export type DeviceRole = "admin" | "member";

/** A device paired with the host. Mirrors `Device` in crates/gateway. */
export interface PairedDevice {
  id: string;
  name: string;
  publicKey: string;
  pairedAt: number;
  lastSeen: number | null;
  role: DeviceRole;
  /** Set in the `devices` call for the device that made it. */
  current?: boolean;
}

/** Who the host thinks this device is (`whoami`). */
export interface DeviceIdentity {
  deviceId: string;
  name: string;
  role: DeviceRole;
}

/** Mirrors `GatewayStatus` in crates/gateway (`access`). */
export interface AccessStatus {
  running: boolean;
  port: number | null;
  addresses: string[];
  hostId: string;
  /** Remote access through a relay, when one is set. */
  relay: RelayStatus | null;
  /** Remote access through a Cloudflare tunnel, when it's on. */
  tunnel: TunnelStatus | null;
  /** Where devices reach the host from anywhere, if anywhere. */
  publicUrl: string | null;
}

/** @deprecated Use AccessStatus. */
export type PhoneAccessStatus = AccessStatus;

export interface TunnelStatus {
  kind: "quick" | "cloudflare";
  url: string | null;
  connected: boolean;
  error: string | null;
}

export type RemoteAccess = "off" | "quick" | "cloudflare";

/** Host settings as admins see them. Mirrors `Settings` in crates/core. */
export interface HostSettings {
  host_name: string | null;
  backend: "auto" | "cpu" | "metal" | "vulkan";
  context_size: number;
  gpu_layers: number;
  llama_server_path: string | null;
  active_model: string | null;
  system_prompt: string;
  disabled_skills: string[];
  phone_port: number;
  relay_url: string | null;
  remote_access: RemoteAccess;
  /** Never sent by the host; send a string to set it, "" to clear it. */
  tunnel_token: string | null;
  tunnel_token_set: boolean;
  public_url: string | null;
  cloudflared_path: string | null;
  check_for_updates: boolean;
  /** Model settings for every chat, from every device. A chat's own options win. */
  chat_defaults: ChatOptions;
  /** Community skill index to browse; null uses the BrainWashed registry. */
  skill_index: string | null;
}

export interface Hardware {
  os: string;
  arch: string;
  total_memory_bytes: number;
  cpu_cores: number;
}

/** A model download in progress or just finished. */
export interface DownloadStatus {
  repo: string;
  done: number;
  total: number | null;
  finished: boolean;
  error: string | null;
}

/** One line of the host's audit log. */
export interface AuditEntry {
  at: number;
  deviceId: string | null;
  deviceName: string | null;
  action: string;
  target?: string;
  error?: string;
}

export interface RelayStatus {
  url: string;
  connected: boolean;
  /** Why the last attempt failed, while not connected. */
  error: string | null;
}

export interface UpdateInfo {
  version: string;
  /** Release page with the installers. */
  url: string;
}

export interface PairingOffer {
  /** App link, `brainwashed://pair?...`. */
  url: string;
  /** The same details as a link to the host's web app. This is what the QR code shows. */
  webUrl: string;
  /** The web link on the local network, if the host is on one. */
  lanUrl: string | null;
  /** The web link through the public address, if there is one. */
  publicUrl: string | null;
  /** What follows `#pair?`. */
  query: string;
  role: DeviceRole;
  expiresAt: number;
  addresses: string[];
  port: number;
  /** The QR code for `webUrl`, as rows of "1" (dark) and "0" (light). Sent by `createPairingOffer`. */
  qr?: string[];
}

export type GatewayEvent = { type: "devicePaired"; device: PairedDevice } | { type: "devicesChanged" };
