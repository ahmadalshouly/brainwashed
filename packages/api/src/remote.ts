// Client for a BrainWashed host's phone gateway. After pairing, every call is
// encrypted end to end with NaCl box using the host key from the QR code.

import nacl from "tweetnacl";
import { fromBase64, toBase64, utf8Decode, utf8Encode } from "./encoding";
import type {
  AccessStatus,
  ApiKey,
  ApiUsage,
  AuditEntry,
  CatalogItem,
  ChatEvent,
  ChatModel,
  ChatOptions,
  ChatMessage,
  DocumentText,
  ProviderInfo,
  ProviderInput,
  DeviceIdentity,
  DeviceRole,
  DownloadStatus,
  EngineState,
  Hardware,
  HostInfo,
  HostSettings,
  InstalledModel,
  NewApiKey,
  PairedDevice,
  PairingOffer,
  SkillInfo,
  SkillList,
  CommunitySkill,
  SkillPreview,
  UpdateInfo,
} from "./types";

export interface PairingInfo {
  hostKey: string;
  token: string;
  addresses: string[];
  port: number;
  hostName: string;
  /** Where to reach the host through its relay when away from home. */
  relay?: string;
  /** The host's public https address (a tunnel or the owner's own domain). */
  publicUrl?: string;
}

const PAIRING_LINK = /^(?:brainwashed:\/\/pair|https?:\/\/[^#]*#pair)\?(.*)$/;

/** Whether a scanned QR code is a BrainWashed pairing link. */
export function isPairingUrl(url: string): boolean {
  return PAIRING_LINK.test(url.trim());
}

/**
 * Parses the pairing link shown as a QR code on the computer: either the app
 * link `brainwashed://pair?...` or the web chat link `http://<host>/#pair?...`.
 * `at` replaces the advertised addresses, for the web chat.
 */
export function parsePairingUrl(url: string, at?: { address: string; port: number }): PairingInfo {
  const match = PAIRING_LINK.exec(url.trim());
  if (!match) throw new Error("This isn't a BrainWashed pairing code.");
  const params = new Map<string, string>();
  for (const kv of match[1].split("&")) {
    const i = kv.indexOf("=");
    if (i > 0) params.set(kv.slice(0, i), decodeURIComponent(kv.slice(i + 1)));
  }
  const get = (k: string) => {
    const v = params.get(k);
    if (!v) throw new Error(`Pairing code is missing "${k}".`);
    return v;
  };
  if (get("v") !== "1") throw new Error("This pairing code needs a newer version of the app.");
  const hostKey = get("k");
  // The web chat can only reach the computer that served it, wherever that is.
  const addresses = at ? [at.address] : (params.get("a") ?? "").split(",").filter(Boolean);
  const relayUrl = at ? undefined : params.get("r")?.replace(/\/+$/, "");
  if (relayUrl !== undefined && !/^https?:\/\/[^/?#\s]+/.test(relayUrl)) {
    throw new Error("This pairing code has a bad relay address.");
  }
  const publicUrl = at ? undefined : params.get("u")?.replace(/\/+$/, "");
  if (publicUrl !== undefined && !/^https?:\/\/[^/?#\s]+$/.test(publicUrl)) {
    throw new Error("This pairing code has a bad public address.");
  }
  if (addresses.length === 0 && !relayUrl && !publicUrl) {
    throw new Error("The computer isn't on a local network. Connect it to Wi-Fi, then show a new code.");
  }
  return {
    hostKey,
    token: get("t"),
    addresses,
    port: at ? at.port : Number(get("p")),
    hostName: params.get("n") ?? "Computer",
    ...(relayUrl ? { relay: `${relayUrl}/h/${hostKey}` } : {}),
    ...(publicUrl ? { publicUrl } : {}),
  };
}

/** Everything a phone stores about a paired host. */
export interface PairedHost {
  hostId: string;
  hostName: string;
  hostKey: string;
  deviceId: string;
  /** This phone's key pair for this host, base64. */
  publicKey: string;
  secretKey: string;
  addresses: string[];
  port: number;
  /** Base URL through the host's relay, tried after the local addresses. */
  relay?: string;
  /** The host's public https address, tried after the local addresses and before the relay. */
  publicUrl?: string;
  /** What the host lets this device do. Older hosts don't say. */
  role?: DeviceRole;
  /** The address (or relay URL) that last worked, tried first. */
  lastAddress?: string;
}

/** Local addresses first, then the public address, then the relay. */
function candidates(h: { addresses: string[]; publicUrl?: string; relay?: string }): string[] {
  return [...h.addresses, ...(h.publicUrl ? [h.publicUrl] : []), ...(h.relay ? [h.relay] : [])];
}

type FetchLike = (url: string, init: { method: string; headers: Record<string, string>; body: string; signal?: AbortSignal }) => Promise<{
  ok: boolean;
  status: number;
  text(): Promise<string>;
  body?: ReadableStream<Uint8Array> | null;
}>;

interface Envelope {
  n: string;
  c: string;
}

function seal(payload: unknown, theirKey: Uint8Array, mySecret: Uint8Array): Envelope {
  const nonce = nacl.randomBytes(nacl.box.nonceLength);
  const c = nacl.box(utf8Encode(JSON.stringify(payload)), nonce, theirKey, mySecret);
  return { n: toBase64(nonce), c: toBase64(c) };
}

function open<T>(env: Envelope, theirKey: Uint8Array, mySecret: Uint8Array): T {
  const plain = nacl.box.open(fromBase64(env.c), fromBase64(env.n), theirKey, mySecret);
  if (!plain) throw new Error("Could not decrypt the computer's reply.");
  return JSON.parse(utf8Decode(plain)) as T;
}

/** The host answered, but with an error. Not worth retrying elsewhere. */
export class HostReplyError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "HostReplyError";
  }
}

async function errorFrom(res: { status: number; text(): Promise<string> }): Promise<Error> {
  const text = await res.text();
  let message = `The computer answered ${res.status}.`;
  try {
    message = JSON.parse(text).error ?? message;
  } catch {
    // Not JSON; keep the generic message.
  }
  return new HostReplyError(message, res.status);
}

/** How long to wait for an address to answer before trying the next one. */
/**
 * For streamed replies. On Android, Expo's fetch holds back brotli-compressed
 * bodies and, in development builds, any body its network inspector doesn't
 * recognize as a stream, so the whole answer would appear at once. Asking for
 * an uncompressed event stream avoids both; browsers ignore Accept-Encoding.
 */
const STREAM_HEADERS = {
  "Content-Type": "application/json",
  Accept: "text/event-stream, application/x-ndjson",
  "Accept-Encoding": "identity",
};

const CONNECT_TIMEOUT_MS = 6000;
/** Away from home, local addresses don't answer; give up on them sooner when a relay can take over. */
const LAN_TIMEOUT_WITH_RELAY_MS = 2500;

/**
 * Extra wait for a big request: about 1 s per 100 KB (a slow upload over
 * mobile data), plus time for the computer to read documents.
 */
function uploadAllowanceMs(method: string, bytes: number): number {
  return (bytes > 64 * 1024 ? Math.round(bytes / 100) : 0) + (method === "readDocument" ? 60_000 : 0);
}

/** A local address, or a full URL (public address or relay), which is used as is. */
function baseUrl(address: string, port: number): string {
  return /^https?:\/\//.test(address) ? address : `http://${address}:${port}`;
}

/**
 * Tries each address in turn and returns the first that answers. Local
 * addresses are host names or IPs; a relay is a full URL.
 * Each attempt gets a signal that aborts if no response arrives in time.
 */
async function reach<T>(
  addresses: string[],
  port: number,
  attempt: (base: string, signal: AbortSignal) => Promise<T>,
  outer?: AbortSignal,
  /** More time for big uploads, such as attached pictures and documents. */
  extraMs = 0,
): Promise<{ result: T; address: string }> {
  const hasRelay = addresses.some((a) => /^https?:\/\//.test(a));
  for (const address of addresses) {
    const isRelay = /^https?:\/\//.test(address);
    const controller = new AbortController();
    const onOuterAbort = () => controller.abort();
    outer?.addEventListener("abort", onOuterAbort);
    const timeout = (hasRelay && !isRelay ? LAN_TIMEOUT_WITH_RELAY_MS : CONNECT_TIMEOUT_MS) + extraMs;
    const timer = setTimeout(() => controller.abort(), timeout);
    try {
      return { result: await attempt(baseUrl(address, port), controller.signal), address };
    } catch (e) {
      // A relay saying the computer is offline is worth reporting as is.
      if (e instanceof HostReplyError && !(isRelay && e.status === 503 && address !== addresses[addresses.length - 1])) throw e;
      if (outer?.aborted) throw e;
      // Network failure or timeout: try the next address.
    } finally {
      clearTimeout(timer);
      outer?.removeEventListener("abort", onOuterAbort);
    }
  }
  throw new Error(
    hasRelay
      ? "Couldn't reach the computer. Check that it's on and BrainWashed is open with device access on."
      : "Couldn't reach the computer. Check that it's on, BrainWashed is open with phone access on, and both are on the same Wi-Fi.",
  );
}

export async function pairWithHost(
  info: PairingInfo,
  deviceName: string,
  fetchImpl: FetchLike = fetch as unknown as FetchLike,
): Promise<PairedHost> {
  const keys = nacl.box.keyPair();
  const hostKey = fromBase64(info.hostKey);
  const env = seal({ token: info.token, deviceName }, hostKey, keys.secretKey);
  const { result, address } = await reach(candidates(info), info.port, async (base, signal) => {
    const res = await fetchImpl(`${base}/pair`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ devicePublicKey: toBase64(keys.publicKey), ...env }),
      signal,
    });
    if (!res.ok) throw await errorFrom(res);
    return open<{ deviceId: string; hostId: string; hostName: string; role?: DeviceRole }>(
      JSON.parse(await res.text()),
      hostKey,
      keys.secretKey,
    );
  });
  return {
    hostId: result.hostId,
    hostName: result.hostName,
    hostKey: toBase64(hostKey),
    deviceId: result.deviceId,
    publicKey: toBase64(keys.publicKey),
    secretKey: toBase64(keys.secretKey),
    addresses: info.addresses,
    port: info.port,
    ...(info.relay ? { relay: info.relay } : {}),
    ...(info.publicUrl ? { publicUrl: info.publicUrl } : {}),
    ...(result.role ? { role: result.role } : {}),
    lastAddress: address,
  };
}

export class RemoteHost {
  private readonly hostKey: Uint8Array;
  private readonly secretKey: Uint8Array;
  private readonly fetchImpl: FetchLike;

  constructor(
    readonly host: PairedHost,
    fetchImpl: FetchLike = fetch as unknown as FetchLike,
    /** Called when a different address than last time worked. */
    private readonly onAddressChange?: (address: string) => void,
  ) {
    this.hostKey = fromBase64(host.hostKey);
    this.secretKey = fromBase64(host.secretKey);
    // Browsers require fetch to be called unbound (not as a method of this class).
    this.fetchImpl = (url, init) => fetchImpl(url, init);
  }

  // Any device.
  info = () => this.call<HostInfo>("info");
  state = () => this.call<EngineState>("state");
  models = () => this.call<InstalledModel[]>("models");
  skills = () => this.call<SkillInfo[]>("skills");
  whoami = () => this.call<DeviceIdentity>("whoami");
  /** The local model (when loaded) and the cloud models this device may use. */
  chatModels = () => this.call<ChatModel[]>("chatModels");
  /** The admin's model settings, used for anything a chat leaves out. */
  chatDefaults = () => this.call<ChatOptions>("chatDefaults");

  // Admins only.
  loadModel = (id: string) => this.call<null>("loadModel", { id });
  unloadModel = () => this.call<null>("unloadModel");
  hardware = () => this.call<Hardware>("hardware");
  catalog = () => this.call<CatalogItem[]>("catalog");
  downloadModel = (repo: string, quant?: string) => this.call<null>("downloadModel", { repo, quant });
  downloads = () => this.call<DownloadStatus[]>("downloads");
  deleteModel = (id: string) => this.call<null>("deleteModel", { id });
  /** Pairs a model with a draft model that speeds it up; null turns it off. */
  setModelSpeedup = (id: string, speedup: string | null) => this.call<null>("setModelSpeedup", { id, speedup });
  setSkillEnabled = (name: string, enabled: boolean) => this.call<null>("setSkillEnabled", { name, enabled });
  skillList = () => this.call<SkillList>("skillList");
  skillSource = (name: string) => this.call<string>("skillSource", { name });
  /** Saves a SKILL.md; returns the skill's name. */
  saveSkill = (source: string, previousName?: string) => this.call<string>("saveSkill", { source, previousName });
  deleteSkill = (name: string) => this.call<null>("deleteSkill", { name });
  /** Skills in the community index. */
  communitySkills = () => this.call<CommunitySkill[]>("communitySkills");
  /** Downloads a skill to read before installing: a community skill's name or a link. */
  previewSkill = (spec: string) => this.call<SkillPreview>("previewSkill", { spec });
  /** Installs what `previewSkill` showed; fails if the file changed since (`sha256`). */
  installSkill = (spec: string, sha256: string, replace = false) =>
    this.call<SkillPreview>("installSkill", { spec, sha256, replace });
  settings = () => this.call<HostSettings>("settings");
  updateSettings = (settings: Partial<HostSettings>) => this.call<HostSettings>("updateSettings", { settings });
  access = () => this.call<AccessStatus>("access");
  checkForUpdate = () => this.call<UpdateInfo | null>("checkForUpdate");
  devices = () => this.call<PairedDevice[]>("devices");
  createPairingOffer = (role: DeviceRole) => this.call<PairingOffer>("createPairingOffer", { role });
  removeDevice = (id: string) => this.call<null>("removeDevice", { id });
  setDeviceRole = (id: string, role: DeviceRole) => this.call<null>("setDeviceRole", { id, role });
  renameDevice = (id: string, name: string) => this.call<null>("renameDevice", { id, name });
  /** Keys for the OpenAI-compatible API. */
  apiKeys = () => this.call<ApiKey[]>("apiKeys");
  createApiKey = (name: string, role: DeviceRole = "member") => this.call<NewApiKey>("createApiKey", { name, role });
  /** API use over the last `days` days, in hours (up to 2 days) or days of this device's time zone. */
  apiUsage = (days = 7) =>
    this.call<ApiUsage>("apiUsage", { days, utcOffset: -new Date().getTimezoneOffset() * 60 });
  revokeApiKey = (id: string) => this.call<null>("revokeApiKey", { id });
  auditLog = (limit = 100) => this.call<AuditEntry[]>("auditLog", { limit });
  providers = () => this.call<ProviderInfo[]>("providers");
  /** Adds or updates a provider. Leave `apiKey` out to keep the saved key, or send "" to remove it. */
  saveProvider = (provider: ProviderInput) => this.call<ProviderInfo>("saveProvider", { provider });
  deleteProvider = (id: string) => this.call<null>("deleteProvider", { id });
  /** Lists a provider's models, to check the address and key. `id` uses the saved key when `apiKey` is left out. */
  providerModels = (baseUrl: string, apiKey?: string, id?: string) =>
    this.call<string[]>("providerModels", { baseUrl, apiKey, id });

  private addresses(): string[] {
    const { lastAddress } = this.host;
    const all = candidates(this.host);
    return lastAddress && all.includes(lastAddress) ? [lastAddress, ...all.filter((a) => a !== lastAddress)] : all;
  }

  private async post(method: string, params: unknown, signal?: AbortSignal, streamed = false) {
    const env = seal({ ts: Date.now(), method, params: params ?? null }, this.hostKey, this.secretKey);
    const body = JSON.stringify({ deviceId: this.host.deviceId, ...env });
    // The attempt's signal times out only until a response arrives; after
    // that it aborts only if the caller's signal does, so long streamed
    // replies are not cut off.
    const { result, address } = await reach(
      this.addresses(),
      this.host.port,
      async (base, attemptSignal) => {
        const res = await this.fetchImpl(`${base}/rpc`, {
          method: "POST",
          headers: streamed ? STREAM_HEADERS : { "Content-Type": "application/json" },
          body,
          signal: attemptSignal,
        });
        if (!res.ok) throw await errorFrom(res);
        return res;
      },
      signal,
      uploadAllowanceMs(method, body.length),
    );
    if (address !== this.host.lastAddress) {
      this.host.lastAddress = address;
      this.onAddressChange?.(address);
    }
    return result;
  }

  async call<T>(method: string, params?: unknown): Promise<T> {
    const res = await this.post(method, params);
    const reply = open<{ ok?: T; error?: string }>(JSON.parse(await res.text()), this.hostKey, this.secretKey);
    if (reply.error !== undefined) throw new Error(reply.error);
    return reply.ok as T;
  }

  /** Reads the text out of a PDF, Word document or text file (base64). */
  readDocument(name: string, data: string): Promise<DocumentText> {
    return this.call("readDocument", { name, data });
  }

  /**
   * Streams a reply; resolves with the full answer.
   *
   * With a `replyId` (letters, digits, `-` and `_`, up to 64), the computer
   * keeps the reply for a while, so if the connection drops the rest can be
   * fetched with `chatResume`. The computer keeps writing the answer either way.
   */
  async chat(
    messages: ChatMessage[],
    onEvent: (e: ChatEvent) => void,
    signal?: AbortSignal,
    /** `model`: "local" or "<provider>/<model>" from `chatModels`; the local model when left out. */
    extra?: { options?: ChatOptions; model?: string; replyId?: string },
  ): Promise<string> {
    return this.readFrames(await this.post("chat", { messages, ...extra }, signal, true), onEvent);
  }

  /**
   * Continues a reply started with `chat` and a `replyId`: the events after
   * the first `after` ones (the count `onEvent` already saw), then the rest
   * as it is written. Rejects if the computer no longer has the reply.
   */
  async chatResume(replyId: string, after: number, onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<string> {
    return this.readFrames(await this.post("chatResume", { replyId, after }, signal, true), onEvent);
  }

  private async readFrames(res: Awaited<ReturnType<FetchLike>>, onEvent: (e: ChatEvent) => void): Promise<string> {
    let answer: string | undefined;
    const handle = (line: string) => {
      if (!line.trim()) return;
      const frame = open<{ event?: ChatEvent; done?: string; error?: string }>(
        JSON.parse(line),
        this.hostKey,
        this.secretKey,
      );
      if (frame.event) onEvent(frame.event);
      if (frame.error !== undefined) throw new Error(frame.error);
      if (frame.done !== undefined) answer = frame.done;
    };

    if (res.body && typeof res.body.getReader === "function") {
      const reader = res.body.getReader();
      let buffer = "";
      let pending = new Uint8Array(0);
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        // Keep any partial UTF-8 sequence for the next chunk by splitting on newline bytes.
        const merged = new Uint8Array(pending.length + value.length);
        merged.set(pending);
        merged.set(value, pending.length);
        const lastNewline = merged.lastIndexOf(10);
        if (lastNewline < 0) {
          pending = merged;
          continue;
        }
        buffer = utf8Decode(merged.subarray(0, lastNewline));
        pending = merged.slice(lastNewline + 1);
        buffer.split("\n").forEach(handle);
      }
      if (pending.length) handle(utf8Decode(pending));
    } else {
      (await res.text()).split("\n").forEach(handle);
    }
    if (answer === undefined) throw new Error("The reply ended unexpectedly.");
    return answer;
  }
}
