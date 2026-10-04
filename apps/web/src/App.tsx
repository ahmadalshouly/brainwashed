import { useCallback, useEffect, useMemo, useState } from "react";
import {
  HostReplyError,
  pairWithHost,
  parsePairingUrl,
  RemoteHost,
  type DeviceRole,
  type EngineState,
  type PairedHost,
} from "@brainwashed/api";
import { ACCENTS, loadHost, loadLook, loadPage, saveChats, saveHost, saveLook, savePage, type Look } from "./storage";
import { errorText, HostContext, stateText, type HostContextValue } from "./ui";
import { ChatPage } from "./pages/Chat";
import { OverviewPage } from "./pages/Overview";
import { ModelsPage } from "./pages/Models";
import { SkillsPage } from "./pages/Skills";
import { DevicesPage } from "./pages/Devices";
import { RemotePage } from "./pages/Remote";
import { SettingsPage } from "./pages/Settings";
import { ActivityPage } from "./pages/Activity";

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
  const where = location.hostname === "localhost" || location.hostname === "127.0.0.1" ? " (this computer)" : "";
  return (device ? `${browser} on ${device}` : browser) + where;
}

/**
 * Pairs using the link in the address bar. The page can only talk to the
 * computer that served it, so it uses this page's own address (local network,
 * tunnel or your own domain), not the ones in the link.
 */
function pairFromLink(link: string): Promise<PairedHost> {
  const port = Number(location.port) || (location.protocol === "https:" ? 443 : 80);
  return pairWithHost(parsePairingUrl(link, { address: location.origin, port }), browserName());
}

function linkHostKey(link: string): string | null {
  const m = /[?&]k=([^&]+)/.exec(link);
  return m ? decodeURIComponent(m[1]) : null;
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
    saveChats([]);
    setHost(null);
    setNotice(why ?? null);
  }, []);

  // A pairing link opened in the browser, on load or while the page is
  // already open: pair once, then drop the secret token from the address bar
  // and history. A browser already paired with this computer keeps its
  // pairing, so opening the admin page again doesn't add a new device.
  useEffect(() => {
    const check = async () => {
      if (!location.hash.startsWith("#pair?")) return;
      const link = location.href;
      history.replaceState(null, "", location.pathname);
      const current = loadHost();
      const key = linkHostKey(link);
      if (current && key && sameKey(current.hostKey, key)) {
        try {
          // An admin has nothing to gain from the link. A member may be
          // opening an admin link, so pairs again.
          const who = await new RemoteHost(current).whoami();
          if (who.role === "admin") {
            const updated = { ...current, role: who.role };
            saveHost(updated);
            setHost(updated);
            return;
          }
        } catch {
          // Not recognized any more: pair again below.
        }
      }
      setPairing(true);
      try {
        const h = await pairFromLink(link);
        saveHost(h);
        saveChats([]);
        setHost(h);
        setNotice(null);
      } catch (e) {
        setNotice(errorText(e));
      } finally {
        setPairing(false);
      }
    };
    check();
    addEventListener("hashchange", check);
    return () => removeEventListener("hashchange", check);
  }, []);

  if (pairing) {
    return (
      <main className="center">
        <div className="spinner" />
        <p className="muted">Connecting to your computer…</p>
      </main>
    );
  }
  if (!host) return <Welcome notice={notice} />;
  return <Shell key={host.deviceId} paired={host} look={look} setLook={setLook} onForget={forget} />;
}

/** Host keys arrive as base64 or base64url. */
function sameKey(a: string, b: string): boolean {
  const norm = (k: string) => k.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  return norm(a) === norm(b);
}

function Welcome({ notice }: { notice: string | null }) {
  return (
    <main className="center welcome">
      <img src="/logo.svg" alt="" width={64} height={64} />
      <h1>BrainWashed</h1>
      {notice && <div className="banner error">{notice}</div>}
      <p>This browser isn't connected to your computer yet.</p>
      <ol>
        <li>
          On the computer running BrainWashed, open the admin page (run <code>brainwashed open</code>).
        </li>
        <li>
          Go to <strong>Devices</strong> and click <strong>Add a device</strong>.
        </li>
        <li>Scan the QR code with this phone's camera, or open the link under it in this browser.</li>
      </ol>
      <p className="muted small">
        Your chats are end-to-end encrypted between this browser and your computer. Nothing is stored in the cloud.
      </p>
    </main>
  );
}

interface NavItem {
  id: string;
  label: string;
  icon: string;
  admin: boolean;
}

const NAV: NavItem[] = [
  { id: "chat", label: "Chat", icon: "M4 5h16v11H8l-4 4z", admin: false },
  { id: "overview", label: "Overview", icon: "M4 4h7v7H4zM13 4h7v4h-7zM13 10h7v10h-7zM4 13h7v7H4z", admin: true },
  { id: "models", label: "Models", icon: "M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5", admin: true },
  { id: "skills", label: "Skills", icon: "M6 3h9l4 4v14H6zM14 3v5h5M9 12h7M9 16h7", admin: true },
  { id: "devices", label: "Devices", icon: "M7 2h10v20H7zM11 18h2", admin: true },
  {
    id: "remote",
    label: "Remote access",
    icon: "M12 3a9 9 0 100 18 9 9 0 000-18zM3 12h18M12 3c3 3.5 3 14.5 0 18M12 3c-3 3.5-3 14.5 0 18",
    admin: true,
  },
  { id: "activity", label: "Activity", icon: "M3 12h4l3-8 4 16 3-8h4", admin: true },
  {
    id: "settings",
    label: "Settings",
    icon: "M12 9a3 3 0 100 6 3 3 0 000-6zM19 12l2-1-1-3-2 .3-1.5-1.5L17 5l-3-1-1 2h-2l-1-2-3 1 .5 2L6 8.5 4 8 3 11l2 1v0l-2 1 1 3 2-.3 1.5 1.5L7 19l3 1 1-2h2l1 2 3-1-.5-2 1.5-1.5 2 .5 1-3z",
    admin: true,
  },
];

function Icon({ d }: { d: string }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" strokeLinecap="round" aria-hidden>
      <path d={d} />
    </svg>
  );
}

function Shell({
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
  const [role, setRole] = useState<DeviceRole>(paired.role ?? "member");
  const [state, setState] = useState<EngineState | null>(null);
  const [offline, setOffline] = useState<string | null>(null);
  const [page, setPage] = useState<string>(() => {
    const saved = loadPage();
    return saved ?? (paired.role === "admin" ? "overview" : "chat");
  });
  const [navOpen, setNavOpen] = useState(false);
  const [prefs, setPrefs] = useState(false);

  const fail = useCallback(
    (e: unknown) => {
      if (e instanceof HostReplyError && e.status === 401) {
        onForget("Your computer no longer recognizes this browser. Connect it again.");
        return;
      }
      setOffline(errorText(e));
    },
    [onForget],
  );

  const refreshState = useCallback(async () => {
    try {
      setState(await remote.state());
      setOffline(null);
    } catch (e) {
      fail(e);
    }
  }, [remote, fail]);

  // Poll quickly while a model loads, slowly otherwise.
  useEffect(() => {
    refreshState();
    const busy = state?.state === "loading" || state?.state === "installingRuntime";
    const timer = setInterval(refreshState, busy ? 1500 : 8000);
    return () => clearInterval(timer);
  }, [refreshState, state?.state]);

  // Roles can change while the page is open.
  useEffect(() => {
    const check = () =>
      remote
        .whoami()
        .then((w) => {
          setRole(w.role);
          if (paired.role !== w.role) {
            paired.role = w.role;
            saveHost(paired);
          }
        })
        .catch(() => {
          // Hosts before roles existed have no whoami; everything was allowed.
          if (!paired.role) setRole("admin");
        });
    check();
    const timer = setInterval(check, 30000);
    return () => clearInterval(timer);
  }, [remote, paired]);

  const items = NAV.filter((n) => !n.admin || role === "admin");
  const current = items.some((n) => n.id === page) ? page : "chat";
  const go = useCallback((p: string) => {
    setPage(p);
    savePage(p);
    setNavOpen(false);
  }, []);

  const ctx: HostContextValue = { remote, paired, role, state, refreshState, fail, go };
  const ready = state?.state === "ready";

  return (
    <HostContext.Provider value={ctx}>
      <div className={`shell ${navOpen ? "nav-open" : ""}`}>
        <aside className="sidebar">
          <div className="brand">
            <img src="/logo.svg" alt="" width={28} height={28} />
            <div>
              <strong>{paired.hostName}</strong>
              <span className={`status-dot ${offline ? "off" : ready ? "on" : "wait"}`}>
                {offline ? "Offline" : stateText(state)}
              </span>
            </div>
          </div>
          <nav>
            {items.map((n) => (
              <button key={n.id} className={current === n.id ? "active" : ""} onClick={() => go(n.id)}>
                <Icon d={n.icon} />
                {n.label}
              </button>
            ))}
          </nav>
          <div className="sidebar-foot">
            <span className="badge">{role === "admin" ? "Admin" : "Member"}</span>
            <button className="ghost" onClick={() => setPrefs(!prefs)}>
              Preferences
            </button>
            {prefs && (
              <div className="prefs">
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
                  className="danger-text"
                  onClick={() => {
                    if (confirm(`Disconnect this browser from ${paired.hostName}?`)) onForget();
                  }}
                >
                  Disconnect this browser
                </button>
              </div>
            )}
          </div>
        </aside>
        <div className="scrim" onClick={() => setNavOpen(false)} />
        <div className="main">
          <div className="topbar">
            <button className="ghost" aria-label="Menu" onClick={() => setNavOpen(true)}>
              <Icon d="M4 6h16M4 12h16M4 18h16" />
            </button>
            <strong>{items.find((n) => n.id === current)?.label}</strong>
          </div>
          {offline && <div className="banner error inset">Can't reach your computer: {offline}</div>}
          {current === "chat" && <ChatPage />}
          {current === "overview" && <OverviewPage />}
          {current === "models" && <ModelsPage />}
          {current === "skills" && <SkillsPage />}
          {current === "devices" && <DevicesPage />}
          {current === "remote" && <RemotePage />}
          {current === "activity" && <ActivityPage />}
          {current === "settings" && <SettingsPage />}
        </div>
      </div>
    </HostContext.Provider>
  );
}
