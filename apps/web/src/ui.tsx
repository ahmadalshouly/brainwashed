// Small pieces shared by the pages.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { DeviceRole, EngineState, PairedHost, RemoteHost } from "@brainwashed/api";

export const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface HostContextValue {
  remote: RemoteHost;
  paired: PairedHost;
  role: DeviceRole;
  state: EngineState | null;
  refreshState: () => void;
  /** Reports an error from a call; forgets the pairing when the host no longer knows this device. */
  fail: (e: unknown) => void;
  go: (page: string) => void;
}

export const HostContext = createContext<HostContextValue | null>(null);

export function useHost(): HostContextValue {
  const ctx = useContext(HostContext);
  if (!ctx) throw new Error("no host");
  return ctx;
}

/**
 * Loads something from the host, again every `every` ms if given.
 * Returns the value, an error, and a function to reload now.
 */
export function useLoad<T>(load: () => Promise<T>, deps: unknown[], every?: number) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const loader = useRef(load);
  loader.current = load;
  const reload = useCallback(async () => {
    try {
      setValue(await loader.current());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, deps);
  useEffect(() => {
    reload();
    if (!every) return;
    const timer = setInterval(reload, every);
    return () => clearInterval(timer);
  }, [reload, every]);
  return { value, error, reload };
}

/** Runs an action, tracking whether it's busy and what went wrong. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = useCallback(async <T,>(f: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    try {
      return await f();
    } catch (e) {
      setError(errorText(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, setError, run };
}

export function formatBytes(n: number | null | undefined): string {
  if (!n) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1000)), units.length - 1);
  return `${(n / 1000 ** i).toFixed(i >= 3 ? 1 : 0)} ${units[i]}`;
}

export function timeAgo(seconds: number | null | undefined): string {
  if (!seconds) return "never";
  const diff = Date.now() / 1000 - seconds;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} d ago`;
  return new Date(seconds * 1000).toLocaleDateString();
}

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="page-header">
      <div>
        <h1>{title}</h1>
        {subtitle && <p className="muted">{subtitle}</p>}
      </div>
      {actions && <div className="actions">{actions}</div>}
    </div>
  );
}

export function Card({ title, children, actions }: { title?: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <section className="card">
      {(title || actions) && (
        <div className="card-head">
          {title && <h2>{title}</h2>}
          {actions && <div className="actions">{actions}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

export function Banner({ kind = "error", children }: { kind?: "error" | "info" | "ok"; children: ReactNode }) {
  return <div className={`banner ${kind}`}>{children}</div>;
}

export function Badge({ kind = "", children }: { kind?: string; children: ReactNode }) {
  return <span className={`badge ${kind}`}>{children}</span>;
}

export function Progress({ done, total }: { done: number; total: number | null }) {
  const pct = total ? Math.min(100, (done / total) * 100) : null;
  return (
    <div className="progress" role="progressbar" aria-valuenow={pct ?? undefined}>
      <div className={pct === null ? "indeterminate" : ""} style={pct === null ? undefined : { width: `${pct}%` }} />
    </div>
  );
}

/** Draws a QR code from rows of "1" and "0". */
export function QrCode({ rows, size = 220 }: { rows: string[]; size?: number }) {
  const n = rows.length;
  const quiet = 4;
  const total = n + quiet * 2;
  const path = rows
    .flatMap((row, y) =>
      [...row].map((c, x) => (c === "1" ? `M${x + quiet} ${y + quiet}h1v1h-1z` : "")).filter(Boolean),
    )
    .join("");
  return (
    <svg className="qr" width={size} height={size} viewBox={`0 0 ${total} ${total}`} shapeRendering="crispEdges">
      <rect width={total} height={total} fill="#fff" />
      <path d={path} fill="#000" />
    </svg>
  );
}

export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      onClick={() =>
        navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        })
      }
    >
      {done ? "Copied" : label}
    </button>
  );
}

export function stateText(state: EngineState | null, modelName?: (id: string) => string): string {
  if (!state) return "Connecting…";
  switch (state.state) {
    case "ready":
      return modelName ? modelName(state.model) : state.model;
    case "loading":
      return "Loading model…";
    case "installingRuntime":
      return "Setting up llama.cpp…";
    case "error":
      return state.message;
    default:
      return "No model running";
  }
}
