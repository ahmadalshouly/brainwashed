// Client for a BrainWashed host's phone gateway. After pairing, every call is
// encrypted end to end with NaCl box using the host key from the QR code.

import nacl from "tweetnacl";
import { fromBase64, toBase64, utf8Decode, utf8Encode } from "./encoding";
import type { ChatEvent, ChatMessage, EngineState, HostInfo, InstalledModel, SkillInfo } from "./types";

export interface PairingInfo {
  hostKey: string;
  token: string;
  addresses: string[];
  port: number;
  hostName: string;
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
  // The web chat can only reach the computer that served it, wherever that is.
  const addresses = at ? [at.address] : (params.get("a") ?? "").split(",").filter(Boolean);
  if (addresses.length === 0) {
    throw new Error("The computer isn't on a local network. Connect it to Wi-Fi, then show a new code.");
  }
  return {
    hostKey: get("k"),
    token: get("t"),
    addresses,
    port: at ? at.port : Number(get("p")),
    hostName: params.get("n") ?? "Computer",
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
  /** The address that last worked, tried first. */
  lastAddress?: string;
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
const CONNECT_TIMEOUT_MS = 6000;

/**
 * Tries each address the host advertised and returns the first that answers.
 * Each attempt gets a signal that aborts if no response arrives in time.
 */
async function reach<T>(
  addresses: string[],
  port: number,
  attempt: (base: string, signal: AbortSignal) => Promise<T>,
  outer?: AbortSignal,
): Promise<{ result: T; address: string }> {
  for (const address of addresses) {
    const controller = new AbortController();
    const onOuterAbort = () => controller.abort();
    outer?.addEventListener("abort", onOuterAbort);
    const timer = setTimeout(() => controller.abort(), CONNECT_TIMEOUT_MS);
    try {
      return { result: await attempt(`http://${address}:${port}`, controller.signal), address };
    } catch (e) {
      if (e instanceof HostReplyError || outer?.aborted) throw e;
      // Network failure or timeout: try the next address.
    } finally {
      clearTimeout(timer);
    }
  }
  throw new Error(
    "Couldn't reach the computer. Check that it's on, BrainWashed is open with phone access on, and both are on the same Wi-Fi.",
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
  const { result, address } = await reach(info.addresses, info.port, async (base, signal) => {
    const res = await fetchImpl(`${base}/pair`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ devicePublicKey: toBase64(keys.publicKey), ...env }),
      signal,
    });
    if (!res.ok) throw await errorFrom(res);
    return open<{ deviceId: string; hostId: string; hostName: string }>(
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

  info = () => this.call<HostInfo>("info");
  state = () => this.call<EngineState>("state");
  models = () => this.call<InstalledModel[]>("models");
  loadModel = (id: string) => this.call<null>("loadModel", { id });
  skills = () => this.call<SkillInfo[]>("skills");
  setSkillEnabled = (name: string, enabled: boolean) => this.call<null>("setSkillEnabled", { name, enabled });

  private addresses(): string[] {
    const { lastAddress, addresses } = this.host;
    return lastAddress ? [lastAddress, ...addresses.filter((a) => a !== lastAddress)] : addresses;
  }

  private async post(method: string, params: unknown, signal?: AbortSignal) {
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
          headers: { "Content-Type": "application/json" },
          body,
          signal: attemptSignal,
        });
        if (!res.ok) throw await errorFrom(res);
        return res;
      },
      signal,
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

  /** Streams a reply; resolves with the full answer. */
  async chat(messages: ChatMessage[], onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<string> {
    const res = await this.post("chat", { messages }, signal);
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
