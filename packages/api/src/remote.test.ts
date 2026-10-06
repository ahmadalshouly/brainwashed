import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import nacl from "tweetnacl";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parsePairingUrl, pairWithHost, RemoteHost, HostReplyError, UnreachableError, type PairedHost } from "./remote";
import { fromBase64, toBase64, utf8Decode, utf8Encode } from "./encoding";

describe("encoding", () => {
  it("round-trips base64 and utf-8", () => {
    for (const s of ["", "a", "ab", "abc", "héllo 🌍 ✓"]) {
      const bytes = utf8Encode(s);
      expect(Array.from(bytes)).toEqual(Array.from(new TextEncoder().encode(s)));
      expect(utf8Decode(bytes)).toBe(s);
      expect(toBase64(bytes)).toBe(Buffer.from(bytes).toString("base64"));
      expect(Array.from(fromBase64(toBase64(bytes)))).toEqual(Array.from(bytes));
    }
    expect(Array.from(fromBase64("-_8"))).toEqual([0xfb, 0xff]);
  });
});

describe("parsePairingUrl", () => {
  it("reads every field", () => {
    const info = parsePairingUrl("brainwashed://pair?v=1&k=abc&t=tok&a=192.168.1.5,10.0.0.2&p=47860&n=Ahmad%27s%20Mac");
    expect(info).toEqual({
      hostKey: "abc",
      token: "tok",
      addresses: ["192.168.1.5", "10.0.0.2"],
      port: 47860,
      hostName: "Ahmad's Mac",
    });
  });

  it("reads the web chat link the QR code shows", () => {
    const info = parsePairingUrl("http://192.168.1.5:47860/#pair?v=1&k=abc&t=tok&a=192.168.1.5&p=47860&n=Mac");
    expect(info).toMatchObject({ hostKey: "abc", token: "tok", addresses: ["192.168.1.5"], port: 47860 });
    // The web chat pairs with whatever served it, even with no LAN address.
    const here = parsePairingUrl("http://localhost:8080/#pair?v=1&k=abc&t=tok&a=&p=47860", {
      address: "localhost",
      port: 8080,
    });
    expect(here).toMatchObject({ addresses: ["localhost"], port: 8080 });
  });

  it("reads the public address", () => {
    const info = parsePairingUrl("brainwashed://pair?v=1&k=abc&t=tok&a=192.168.1.5&p=1&u=https%3A%2F%2Fai.example.org%2F");
    expect(info.publicUrl).toBe("https://ai.example.org");
    // The web app stays on whatever address served it.
    const web = parsePairingUrl("https://ai.example.org/#pair?v=1&k=abc&t=tok&a=&p=1&u=https%3A%2F%2Fai.example.org", {
      address: "https://ai.example.org",
      port: 443,
    });
    expect(web.publicUrl).toBeUndefined();
    expect(web.addresses).toEqual(["https://ai.example.org"]);
    expect(() => parsePairingUrl("brainwashed://pair?v=1&k=abc&t=tok&a=&p=1&u=javascript%3Aalert(1)")).toThrow();
  });

  it("reads where to look the computer up when its address changes", () => {
    const lookup = "https://book.example/a/ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0";
    const info = parsePairingUrl(`brainwashed://pair?v=1&k=abc&t=tok&a=&p=1&u=https%3A%2F%2Fa.example&b=${encodeURIComponent(lookup)}`);
    expect(info.lookup).toBe(lookup);
    expect(() => parsePairingUrl("brainwashed://pair?v=1&k=abc&t=tok&a=1.2.3.4&p=1&b=javascript%3Aalert(1)")).toThrow(/address book/);
    // The web app can only reach the address that served it.
    expect(parsePairingUrl(`https://a.example/#pair?v=1&k=abc&t=tok&a=&p=1&b=${encodeURIComponent(lookup)}`, { address: "https://a.example", port: 443 }).lookup).toBeUndefined();
  });

  it("reads the relay address", () => {
    const info = parsePairingUrl("brainwashed://pair?v=1&k=abc_-&t=tok&a=&p=1&r=https%3A%2F%2Frelay.example.org%2F");
    expect(info.relay).toBe("https://relay.example.org/h/abc_-");
    expect(info.addresses).toEqual([]);
    // The web chat stays on the network that served it.
    const web = parsePairingUrl("http://10.0.0.2:1/#pair?v=1&k=abc&t=tok&a=&p=1&r=https%3A%2F%2Frelay.example.org", {
      address: "10.0.0.2",
      port: 1,
    });
    expect(web.relay).toBeUndefined();
    expect(() => parsePairingUrl("brainwashed://pair?v=1&k=a&t=b&a=&p=1&r=javascript%3Aalert(1)")).toThrow(/relay/);
  });

  it("explains a host with no network address", () => {
    expect(() => parsePairingUrl("brainwashed://pair?v=1&k=a&t=b&a=&p=1")).toThrow(/local network/);
  });

  it("rejects other QR codes and newer versions", () => {
    expect(() => parsePairingUrl("https://example.com")).toThrow(/isn't a BrainWashed/);
    expect(() => parsePairingUrl("brainwashed://pair?v=2&k=a&t=b&a=c&p=1")).toThrow(/newer version/);
  });
});

// Runs against the real Rust gateway when its dev server is built:
// cargo build -p brainwashed-gateway --example devserver
// BRAINWASHED_DEVSERVER=target/debug/examples/devserver pnpm test
const devserver = process.env.BRAINWASHED_DEVSERVER;

describe.skipIf(!devserver)("against the real gateway", () => {
  let child: ChildProcess;
  let url: string;
  let paired: PairedHost;

  beforeAll(async () => {
    const dir = mkdtempSync(join(tmpdir(), "bw-gw-"));
    child = spawn(devserver!, [dir], { stdio: ["ignore", "pipe", "inherit"] });
    url = await new Promise<string>((resolve, reject) => {
      let out = "";
      child.stdout!.on("data", (d) => {
        out += d;
        const line = out.split("\n").find((l) => l.startsWith("{"));
        // CI machines may have no LAN address; loopback stands in for one.
        if (line) resolve(JSON.parse(line).url.replace("&a=&", "&a=127.0.0.1&"));
      });
      child.on("exit", (code) => reject(new Error(`devserver exited ${code}`)));
    });
  }, 60_000);

  afterAll(() => {
    child?.kill("SIGINT");
  });

  it("pairs, calls and streams chat", async () => {
    const info = parsePairingUrl(url);
    // An unreachable address first, to exercise fallback.
    info.addresses = ["192.0.2.1", "127.0.0.1"]; // 192.0.2.1 is reserved and never answers
    const host = await pairWithHost(info, "Vitest phone");
    paired = host;
    expect(host.lastAddress).toBe("127.0.0.1");
    expect(host.hostId).toHaveLength(16);

    const remote = new RemoteHost(host);
    const hostInfo = await remote.info();
    expect(hostInfo.version).toBeTruthy();
    const skills = await remote.skills();
    expect(skills.map((s) => s.name)).toContain("writing-assistant");

    await remote.setSkillEnabled("writing-assistant", false);
    expect((await remote.skills()).find((s) => s.name === "writing-assistant")?.enabled).toBe(false);

    const events: string[] = [];
    const chat = remote.chat([{ role: "user", content: "Draft an email to my landlord" }], (e) => events.push(e.kind));
    if (process.env.BRAINWASHED_TEST_MODEL) {
      await expect(chat).resolves.toBeTypeOf("string");
      expect(events[0]).toBe("skills");
      expect(events.length).toBeGreaterThan(1);
    } else {
      await expect(chat).rejects.toThrow(/no model/);
    }

    // A kept reply can be fetched again, as after a dropped connection.
    const kept = remote.chat([{ role: "user", content: "Hi" }], () => {}, undefined, { replyId: "vitest-1" });
    const again = remote.chatResume("vitest-1", 0, () => {});
    if (process.env.BRAINWASHED_TEST_MODEL) {
      await expect(again).resolves.toBe(await kept);
    } else {
      await expect(kept).rejects.toThrow(/no model/);
      await expect(again).rejects.toThrow(/no model/);
    }
    await expect(remote.chatResume("unknown", 0, () => {})).rejects.toThrow(/no longer/);
  }, 60_000);

  it("falls back to the relay when the computer's addresses don't answer", async () => {
    expect(paired.relay).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/h\/[\w-]+$/);
    // As if away from home: the local address no longer answers.
    const away = new RemoteHost({ ...paired, addresses: ["192.0.2.1"], lastAddress: undefined });
    expect((await away.info()).version).toBeTruthy();
    expect(away.host.lastAddress).toBe(paired.relay);

    const chat = away.chat([{ role: "user", content: "Hi" }], () => {});
    if (process.env.BRAINWASHED_TEST_MODEL) {
      await expect(chat).resolves.toBeTypeOf("string");
    } else {
      await expect(chat).rejects.toThrow(/no model/);
    }
  }, 60_000);

  it("refuses a reused pairing code", async () => {
    const info = parsePairingUrl(url);
    info.addresses = ["127.0.0.1"];
    await expect(pairWithHost(info, "Second phone")).rejects.toBeInstanceOf(HostReplyError);
  });
});

describe("RemoteHost", () => {
  it("calls fetch unbound, as browsers require", async () => {
    const calls: unknown[] = [];
    function strictFetch(this: unknown) {
      calls.push(this);
      return Promise.reject(new TypeError("offline"));
    }
    const remote = new RemoteHost(
      {
        hostId: "h",
        hostName: "h",
        hostKey: toBase64(new Uint8Array(32)),
        deviceId: "d",
        publicKey: toBase64(new Uint8Array(32)),
        secretKey: toBase64(new Uint8Array(32)),
        addresses: ["127.0.0.1"],
        port: 1,
      },
      strictFetch as never,
    );
    await expect(remote.info()).rejects.toThrow(/Couldn't reach/);
    expect(calls).toEqual([undefined]);
  });
});

describe("finding a computer whose address changed", () => {
  const hostKeys = nacl.box.keyPair();
  const phone = nacl.box.keyPair();
  const lookup = "https://book.example/a/id";
  const host = (): PairedHost => ({
    hostId: "h",
    hostName: "h",
    hostKey: toBase64(hostKeys.publicKey),
    deviceId: "d",
    publicKey: toBase64(phone.publicKey),
    secretKey: toBase64(phone.secretKey),
    addresses: ["192.0.2.1"],
    port: 1,
    publicUrl: "https://old.example",
    lookup,
  });
  /** The computer now answers at `now`; everything else is offline. */
  function network(now: string | null, book: unknown = { url: now }) {
    const asked: string[] = [];
    const fetchImpl = async (url: string) => {
      asked.push(url);
      if (url === lookup) return { ok: book !== null, status: book ? 200 : 404, text: async () => JSON.stringify(book) };
      if (now && url === `${now}/rpc`) {
        const nonce = nacl.randomBytes(24);
        const c = nacl.box(utf8Encode(JSON.stringify({ ok: { version: "9.9.9" } })), nonce, phone.publicKey, hostKeys.secretKey);
        return { ok: true, status: 200, text: async () => JSON.stringify({ n: toBase64(nonce), c: toBase64(c) }) };
      }
      throw new TypeError("offline");
    };
    return { asked, fetchImpl: fetchImpl as never };
  }

  it("looks it up in the address book and remembers the new address", async () => {
    const { asked, fetchImpl } = network("https://new.example");
    const saved: PairedHost[] = [];
    const addresses: string[] = [];
    const remote = new RemoteHost(host(), fetchImpl, (a) => addresses.push(a), (h) => saved.push(h));
    expect(((await remote.info()) as { version: string }).version).toBe("9.9.9");
    expect(asked).toEqual(["http://192.0.2.1:1/rpc", "https://old.example/rpc", lookup, "https://new.example/rpc"]);
    expect(remote.host.publicUrl).toBe("https://new.example");
    expect(addresses).toEqual(["https://new.example"]);
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatchObject({ publicUrl: "https://new.example", lastAddress: "https://new.example" });

    // Next time the new address is tried first, with no lookup.
    asked.length = 0;
    await remote.info();
    expect(asked).toEqual(["https://new.example/rpc"]);
  });

  it("says the computer is offline when the address book has nothing new", async () => {
    for (const book of [{ url: "https://old.example" }, null, { url: "javascript:alert(1)" }]) {
      const { asked, fetchImpl } = network(null, book);
      const saved: PairedHost[] = [];
      const remote = new RemoteHost(host(), fetchImpl, undefined, (h) => saved.push(h));
      await expect(remote.info()).rejects.toBeInstanceOf(UnreachableError);
      expect(asked.at(-1)).toBe(lookup);
      expect(saved).toEqual([]);
    }
  });

  it("doesn't look anything up without an address book", async () => {
    const { asked, fetchImpl } = network("https://new.example");
    const remote = new RemoteHost({ ...host(), lookup: undefined }, fetchImpl);
    await expect(remote.info()).rejects.toThrow(/Couldn't reach/);
    expect(asked).not.toContain(lookup);
  });
});

describe("stopping", () => {
  it("stops a streamed reply at once and closes the request", async () => {
    const hostKeys = nacl.box.keyPair();
    const deviceKeys = nacl.box.keyPair();
    const frame = (payload: unknown) => {
      const n = nacl.randomBytes(nacl.box.nonceLength);
      const c = nacl.box(utf8Encode(JSON.stringify(payload)), n, deviceKeys.publicKey, hostKeys.secretKey);
      return utf8Encode(JSON.stringify({ n: toBase64(n), c: toBase64(c) }) + "\n");
    };
    let requestSignal: AbortSignal | undefined;
    const fakeFetch = (_url: string, init: { signal?: AbortSignal }) => {
      requestSignal = init.signal;
      // One word, then the computer keeps going without sending more.
      const body = new ReadableStream<Uint8Array>({
        start(c) {
          c.enqueue(frame({ event: { kind: "content", text: "Hel" } }));
        },
      });
      return Promise.resolve({ ok: true, status: 200, text: async () => "", body });
    };
    const remote = new RemoteHost(
      {
        hostId: "h",
        hostName: "h",
        hostKey: toBase64(hostKeys.publicKey),
        deviceId: "d",
        publicKey: toBase64(deviceKeys.publicKey),
        secretKey: toBase64(deviceKeys.secretKey),
        addresses: ["127.0.0.1"],
        port: 1,
      },
      fakeFetch as never,
    );
    const controller = new AbortController();
    const seen: string[] = [];
    const reply = remote.chat([{ role: "user", content: "hi" }], (e) => {
      if (e.kind === "content") seen.push(e.text);
      controller.abort();
    }, controller.signal);
    await expect(reply).rejects.toThrow(/Stopped/);
    expect(seen).toEqual(["Hel"]);
    expect(requestSignal?.aborted).toBe(true);
  });
});
