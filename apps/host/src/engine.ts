import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type {
  CatalogItem,
  ChatMessage,
  Delta,
  EngineEvent,
  EngineState,
  HostInfo,
  InstalledModel,
} from "@brainwashed/api";

export const engine = {
  info: () => invoke<HostInfo>("host_info"),
  state: () => invoke<EngineState>("engine_state"),
  catalog: () => invoke<CatalogItem[]>("catalog"),
  models: () => invoke<InstalledModel[]>("models"),
  download: (repo: string, quant?: string) =>
    invoke<InstalledModel>("download_model", { repo, quant: quant ?? null }),
  importModel: (path: string) => invoke<InstalledModel>("import_model", { path }),
  deleteModel: (id: string) => invoke<void>("delete_model", { id }),
  load: (id: string) => invoke<void>("load_model", { id }),
  unload: () => invoke<void>("unload_model"),
  chat(messages: ChatMessage[], onDelta: (d: Delta) => void) {
    const channel = new Channel<Delta>();
    channel.onmessage = onDelta;
    return invoke<string>("chat", { messages, onDelta: channel });
  },
};

export function onEngineEvent(handler: (e: EngineEvent) => void) {
  const unlisten = listen<EngineEvent>("engine", (e) => handler(e.payload));
  return () => {
    unlisten.then((f) => f());
  };
}

/** The engine's current state, kept live. */
export function useEngineState(): EngineState {
  const [state, setState] = useState<EngineState>({ state: "idle" });
  useEffect(() => {
    engine.state().then(setState);
    return onEngineEvent((e) => {
      if (e.type === "state") {
        const { type: _, ...rest } = e;
        setState(rest as EngineState);
      }
    });
  }, []);
  return state;
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(0)} MB`;
  return `${Math.round(n / 1e3)} KB`;
}

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
