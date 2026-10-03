import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  HostReplyError,
  pairWithHost,
  parsePairingUrl,
  RemoteHost,
  type ChatMessage,
  type EngineState,
  type InstalledModel,
  type PairedHost,
} from "@brainwashed/api";
import { ACCENTS, loadChat, loadHost, loadLook, saveChat, saveHost, saveLook, type Look } from "./storage";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** A readable name for this browser in the computer's device list. */
function browserName(): string {
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua)
    ? "Edge"
    : /Firefox\//.test(ua)
      ? "Firefox"
      : /Chrome\//.test(ua)
        ? "Chrome"
        : /Safari\//.test(ua)
          ? "Safari"
          : "Browser";
  const device = /iPhone/.test(ua)
    ? "iPhone"
    : /iPad/.test(ua)
      ? "iPad"
      : /Android/.test(ua)
        ? "Android"
        : /Mac OS X/.test(ua)
          ? "Mac"
          : /Windows/.test(ua)
            ? "Windows"
            : /Linux/.test(ua)
              ? "Linux"
              : "";
  return device ? `${browser} on ${device}` : browser;
}

/**
 * Pairs using the link in the address bar. The page can only talk to the
 * computer that served it, so it uses this page's address, not the ones in
 * the link.
 */
function pairFromLink(link: string): Promise<PairedHost> {
  const port = Number(location.port) || (location.protocol === "https:" ? 443 : 80);
  return pairWithHost(parsePairingUrl(link, { address: location.hostname, port }), browserName());
}

function useLook(): [Look, (l: Look) => void] {
  const [look, setLook] = useState(loadLook);
  useEffect(() => {
    const root = document.documentElement;
    if (look.theme === "auto") delete root.dataset.theme;
    else root.dataset.theme = look.theme;
    root.style.setProperty("--accent", look.accent);
    saveLook(look);
  }, [look]);
  return [look, setLook];
}

export function App() {
  const [host, setHost] = useState<PairedHost | null>(loadHost);
  const [pairing, setPairing] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [look, setLook] = useLook();

  const forget = useCallback((why?: string) => {
    saveHost(null);
    saveChat([]);
    setHost(null);
    setNotice(why ?? null);
  }, []);

  // A pairing link opened in the browser, on load or while the page is
  // already open: pair once, then drop the secret token from the address bar
  // and history.
  useEffect(() => {
    const check = () => {
      if (!location.hash.startsWith("#pair?")) return;
      const link = location.href;
      history.replaceState(null, "", location.pathname);
      setPairing(true);
      pairFromLink(link)
        .then((h) => {
          saveHost(h);
          saveChat([]);
          setHost(h);
          setNotice(null);
        })
        .catch((e) => setNotice(errorText(e)))
        .finally(() => setPairing(false));
    };
    check();
    addEventListener("hashchange", check);
    return () => removeEventListener("hashchange", check);
  }, []);

  if (pairing) {
    return (
      <main className="center">
        <p className="muted">Pairing with your computer…</p>
      </main>
    );
  }
  if (!host) return <Welcome notice={notice} />;
  return <Chat key={host.deviceId} paired={host} look={look} setLook={setLook} onForget={forget} />;
}

function Welcome({ notice }: { notice: string | null }) {
  return (
    <main className="center welcome">
      <img src="/logo.svg" alt="" width={56} height={56} />
      <h1>BrainWashed</h1>
      {notice && <div className="banner error">{notice}</div>}
      <p>This browser isn't paired with your computer yet.</p>
      <ol>
        <li>
          On your computer, open BrainWashed and go to <strong>Devices</strong>.
        </li>
        <li>
          Click <strong>Pair a device</strong>.
        </li>
        <li>Scan the QR code with this phone's camera, or open the link under it in this browser.</li>
      </ol>
      <p className="muted small">
        Your chats stay between this browser and your computer. Nothing goes through the internet.
      </p>
    </main>
  );
}

interface Turn extends ChatMessage {
  skills?: string[];
  reasoning?: string;
  error?: string;
}

function Chat({
  paired,
  look,
  setLook,
  onForget,
}: {
  paired: PairedHost;
  look: Look;
  setLook: (l: Look) => void;
  onForget: (why?: string) => void;
}) {
  const remote = useMemo(() => new RemoteHost(paired, fetch, () => saveHost(paired)), [paired]);
  const [state, setState] = useState<EngineState | null>(null);
  const [models, setModels] = useState<InstalledModel[]>([]);
  const [offline, setOffline] = useState<string | null>(null);
  const [turns, setTurns] = useState<Turn[]>(loadChat);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const bottom = useRef<HTMLDivElement>(null);

  // The computer removed this browser: start over.
  const handle = useCallback(
    (e: unknown) => {
      if (e instanceof HostReplyError && e.status === 401) {
        onForget("Your computer no longer recognizes this browser. Pair it again.");
        return;
      }
      setOffline(errorText(e));
    },
    [onForget],
  );

  const refresh = useCallback(async () => {
    try {
      const [s, m] = await Promise.all([remote.state(), remote.models()]);
      setState(s);
      setModels(m);
      setOffline(null);
    } catch (e) {
      handle(e);
    }
  }, [remote, handle]);

  // Poll quickly while a model loads, slowly otherwise.
  useEffect(() => {
    refresh();
    const loading = state?.state === "loading" || state?.state === "installingRuntime";
    const timer = setInterval(refresh, loading ? 1500 : 10000);
    return () => clearInterval(timer);
  }, [refresh, state?.state]);

  useEffect(() => {
    saveChat(turns.filter((t) => !t.error).map(({ role, content }) => ({ role, content })));
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [turns]);

  const ready = state?.state === "ready";
  const current = state && "model" in state ? state.model : null;

  async function send() {
    const text = input.trim();
    if (!text || busy || !ready) return;
    const history: ChatMessage[] = [
      ...turns.filter((t) => !t.error).map(({ role, content }) => ({ role, content })),
      { role: "user", content: text },
    ];
    setTurns([...history, { role: "assistant", content: "" }]);
    setInput("");
    setBusy(true);
    const controller = new AbortController();
    abort.current = controller;
    const update = (f: (t: Turn) => Turn) => setTurns((all) => [...all.slice(0, -1), f(all[all.length - 1])]);
    try {
      await remote.chat(
        history,
        (e) =>
          update((t) => {
            switch (e.kind) {
              case "skills":
                return { ...t, skills: e.names };
              case "content":
                return { ...t, content: t.content + e.text };
              case "reasoning":
                return { ...t, reasoning: (t.reasoning ?? "") + e.text };
            }
          }),
        controller.signal,
      );
    } catch (e) {
      if (!controller.signal.aborted) {
        if (e instanceof HostReplyError && e.status === 401) handle(e);
        else update((t) => ({ ...t, error: errorText(e) }));
      }
    } finally {
      abort.current = null;
      setBusy(false);
    }
  }

  async function pickModel(id: string) {
    try {
      await remote.loadModel(id);
      setState({ state: "loading", model: id });
    } catch (e) {
      handle(e);
    }
  }

  const status = offline
    ? "Can't reach your computer"
    : !state
      ? "Connecting…"
      : state.state === "ready"
        ? (models.find((m) => m.id === state.model)?.name ?? state.model)
        : state.state === "loading"
          ? "Loading model…"
          : state.state === "installingRuntime"
            ? "Setting up…"
            : state.state === "error"
              ? state.message
              : "No model running";

  return (
    <div className="app">
      <header>
        <div className="title">
          <strong>{paired.hostName}</strong>
          <span className={`muted small ${offline ? "danger" : ""}`}>{status}</span>
        </div>
        {models.length > 0 && (
          <select
            aria-label="Model"
            value={current ?? ""}
            disabled={busy || state?.state === "loading"}
            onChange={(e) => pickModel(e.target.value)}
          >
            {!current && <option value="">Choose a model</option>}
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </select>
        )}
        <button aria-label="Settings" onClick={() => setMenu(!menu)}>
          ⋯
        </button>
      </header>

      {menu && (
        <section className="menu">
          <label>
            Theme
            <select value={look.theme} onChange={(e) => setLook({ ...look, theme: e.target.value as Look["theme"] })}>
              <option value="auto">Match device</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
          <div className="accents" role="radiogroup" aria-label="Accent color">
            {ACCENTS.map((c) => (
              <button
                key={c}
                role="radio"
                aria-checked={look.accent === c}
                aria-label={c}
                className={look.accent === c ? "swatch on" : "swatch"}
                style={{ background: c }}
                onClick={() => setLook({ ...look, accent: c })}
              />
            ))}
          </div>
          <button
            onClick={() => {
              if (confirm(`Unpair this browser from ${paired.hostName}?`)) onForget();
            }}
          >
            Unpair this browser
          </button>
        </section>
      )}

      <main className="messages">
        {offline && <div className="banner error">{offline}</div>}
        {turns.length === 0 && (
          <p className="hint">
            {ready
              ? "Ask anything. Everything stays on your computer."
              : models.length === 0 && state
                ? "Download a model in BrainWashed on your computer to start chatting."
                : "Pick a model above to start chatting."}
          </p>
        )}
        {turns.map((t, i) => (
          <div key={i} className={`bubble ${t.role}`}>
            {t.skills && t.skills.length > 0 && <div className="skills-used">Using {t.skills.join(", ")}</div>}
            {t.reasoning && (
              <details className="reasoning">
                <summary>Thinking</summary>
                {t.reasoning}
              </details>
            )}
            {t.content || (busy && i === turns.length - 1 ? <span className="typing">…</span> : null)}
            {t.error && <div className="error">{t.error}</div>}
          </div>
        ))}
        <div ref={bottom} />
      </main>

      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <textarea
          value={input}
          placeholder={ready ? "Message BrainWashed" : "Load a model to chat"}
          disabled={!ready}
          rows={1}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !matchMedia("(pointer: coarse)").matches) {
              e.preventDefault();
              send();
            }
          }}
        />
        {busy ? (
          <button type="button" onClick={() => abort.current?.abort()}>
            Stop
          </button>
        ) : (
          <button className="primary" type="submit" disabled={!ready || !input.trim()}>
            Send
          </button>
        )}
        {turns.length > 0 && !busy && (
          <button type="button" onClick={() => setTurns([])}>
            New
          </button>
        )}
      </form>
    </div>
  );
}
