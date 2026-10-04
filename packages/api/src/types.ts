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
