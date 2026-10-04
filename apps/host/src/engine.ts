import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type {
  CatalogItem,
  ChatEvent,
  ChatMessage,
  EngineEvent,
  EngineState,
  HostInfo,
  GatewayEvent,
  InstalledModel,
  PairedDevice,
  PairingOffer,
  PhoneAccessStatus,
  UpdateInfo,
  SkillList,
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
  skills: () => invoke<SkillList>("skills"),
  skillSource: (name: string) => invoke<string>("skill_source", { name }),
  saveSkill: (source: string, previousName?: string) =>
    invoke<string>("save_skill", { source, previousName: previousName ?? null }),
  deleteSkill: (name: string) => invoke<void>("delete_skill", { name }),
  setSkillEnabled: (name: string, enabled: boolean) =>
    invoke<void>("set_skill_enabled", { name, enabled }),
  checkForUpdate: () => invoke<UpdateInfo | null>("check_for_update"),
  updateChecksEnabled: () => invoke<boolean>("update_checks_enabled"),
  setUpdateChecks: (enabled: boolean) => invoke<void>("set_update_checks", { enabled }),
  openReleasePage: (url: string) => invoke<void>("open_release_page", { url }),
  phoneStatus: () => invoke<PhoneAccessStatus>("phone_status"),
  setPhoneAccess: (enabled: boolean) => invoke<PhoneAccessStatus>("set_phone_access", { enabled }),
  setRelayUrl: (url: string | null) => invoke<PhoneAccessStatus>("set_relay_url", { url }),
  createPairingOffer: () => invoke<PairingOffer>("create_pairing_offer"),
  devices: () => invoke<PairedDevice[]>("paired_devices"),
  removeDevice: (id: string) => invoke<void>("remove_device", { id }),
  chat(messages: ChatMessage[], onEvent: (e: ChatEvent) => void) {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<string>("chat", { messages, onEvent: channel });
  },
};

export function onEngineEvent(handler: (e: EngineEvent) => void) {
  const unlisten = listen<EngineEvent>("engine", (e) => handler(e.payload));
  return () => {
    unlisten.then((f) => f());
  };
}

export function onGatewayEvent(handler: (e: GatewayEvent) => void) {
  const unlisten = listen<GatewayEvent>("gateway", (e) => handler(e.payload));
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
