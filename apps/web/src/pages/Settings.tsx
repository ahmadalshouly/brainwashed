import { useEffect, useState } from "react";
import type { ChatOptions, HostSettings } from "@brainwashed/api";
import { Banner, Card, PageHeader, useAction, useHost, useLoad } from "../ui";

export function SettingsPage() {
  const { remote } = useHost();
  const loaded = useLoad<HostSettings>(() => remote.settings(), [remote]);
  const action = useAction();
  const [s, setS] = useState<HostSettings | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => setS(loaded.value), [loaded.value]);
  if (!s) return <div className="page">{loaded.error ? <Banner>{loaded.error}</Banner> : <p className="muted">Loading…</p>}</div>;

  const set = <K extends keyof HostSettings>(k: K, v: HostSettings[K]) => {
    setSaved(false);
    setS({ ...s, [k]: v });
  };

  async function save() {
    if (!s) return;
    const patch: Partial<HostSettings> = {
      host_name: s.host_name?.trim() || null,
      system_prompt: s.system_prompt,
      context_size: s.context_size,
      gpu_layers: s.gpu_layers,
      backend: s.backend,
      llama_server_path: s.llama_server_path?.trim() || null,
      cloudflared_path: s.cloudflared_path?.trim() || null,
      phone_port: s.phone_port,
      check_for_updates: s.check_for_updates,
      chat_defaults: s.chat_defaults ?? {},
    };
    const next = await action.run(() => remote.updateSettings(patch));
    if (next) {
      loaded.reload();
      setSaved(true);
    }
  }

  return (
    <div className="page">
      <PageHeader
        title="Settings"
        actions={
          <button className="primary" disabled={action.busy} onClick={save}>
            Save
          </button>
        }
      />
      {action.error && <Banner>{action.error}</Banner>}
      {saved && !action.error && <Banner kind="ok">Saved. Chat defaults apply to the next message; runtime settings apply the next time a model loads.</Banner>}

      <Card title="General">
        <div className="form">
          <label>
            Computer name
            <input placeholder="Shown on devices" value={s.host_name ?? ""} onChange={(e) => set("host_name", e.target.value)} />
          </label>
          <label className="check">
            <input type="checkbox" checked={s.check_for_updates} onChange={(e) => set("check_for_updates", e.target.checked)} />
            Check GitHub for new versions
          </label>
        </div>
      </Card>

      <Card title="Assistant">
        <div className="form">
          <label>
            System prompt
            <textarea rows={5} value={s.system_prompt} onChange={(e) => set("system_prompt", e.target.value)} />
            <span className="muted small">Instructions the model gets before every conversation, for everyone.</span>
          </label>
        </div>
      </Card>

      <Card title="Chat defaults">
        <div className="form">
          <span className="muted small">
            Used by every chat on every device, including the phone apps. A chat can still change them for itself. Empty fields use the model's own defaults.
          </span>
          <label>
            Thinking
            <select
              value={s.chat_defaults?.reasoning === undefined ? "" : s.chat_defaults.reasoning ? "on" : "off"}
              onChange={(e) =>
                set("chat_defaults", { ...s.chat_defaults, reasoning: e.target.value === "" ? undefined : e.target.value === "on" })
              }
            >
              <option value="">Model default</option>
              <option value="on">On: think before answering</option>
              <option value="off">Off: answer right away (faster)</option>
            </select>
            <span className="muted small">For reasoning models such as Qwen3 and DeepSeek R1.</span>
          </label>
          <div className="form two">
            <DefaultNumber label="Temperature" hint="0 to 2. Lower is focused, higher is creative." field="temperature" s={s} set={set} step={0.05} />
            <DefaultNumber label="Longest reply (tokens)" hint="Leave empty for no limit." field="maxTokens" s={s} set={set} step={64} integer />
            <DefaultNumber label="Top P" hint="0 to 1." field="topP" s={s} set={set} step={0.05} />
            <DefaultNumber label="Top K" hint="Up to 1000. Local models only." field="topK" s={s} set={set} step={1} integer />
            <DefaultNumber label="Min P" hint="0 to 1. Local models only." field="minP" s={s} set={set} step={0.01} />
            <DefaultNumber label="Repeat penalty" hint="0.5 to 2. Local models only." field="repeatPenalty" s={s} set={set} step={0.05} />
            <DefaultNumber label="Presence penalty" hint="-2 to 2." field="presencePenalty" s={s} set={set} step={0.1} />
            <DefaultNumber label="Seed" hint="The same seed repeats the same reply." field="seed" s={s} set={set} step={1} integer />
          </div>
        </div>
      </Card>

      <Card title="Model runtime">
        <div className="form two">
          <label>
            Context size (tokens)
            <input type="number" min={512} step={512} value={s.context_size} onChange={(e) => set("context_size", Number(e.target.value))} />
          </label>
          <label>
            GPU layers
            <input type="number" min={0} value={s.gpu_layers} onChange={(e) => set("gpu_layers", Number(e.target.value))} />
            <span className="muted small">999 puts the whole model on the GPU; 0 uses only the CPU.</span>
          </label>
          <label>
            Backend
            <select value={s.backend} onChange={(e) => set("backend", e.target.value as HostSettings["backend"])}>
              <option value="auto">Automatic</option>
              <option value="cpu">CPU</option>
              <option value="metal">Metal (Apple)</option>
              <option value="vulkan">Vulkan (most GPUs)</option>
            </select>
          </label>
          <label>
            Custom llama-server
            <input placeholder="Downloaded automatically" value={s.llama_server_path ?? ""} onChange={(e) => set("llama_server_path", e.target.value)} />
          </label>
        </div>
      </Card>

      <Card title="Network">
        <div className="form two">
          <label>
            Port
            <input type="number" min={1} max={65535} value={s.phone_port} onChange={(e) => set("phone_port", Number(e.target.value))} />
            <span className="muted small">Takes effect when BrainWashed restarts.</span>
          </label>
          <label>
            Custom cloudflared
            <input placeholder="Downloaded automatically" value={s.cloudflared_path ?? ""} onChange={(e) => set("cloudflared_path", e.target.value)} />
          </label>
        </div>
      </Card>
    </div>
  );
}

type NumberField = Exclude<keyof ChatOptions, "reasoning">;

function DefaultNumber({
  label,
  hint,
  field,
  s,
  set,
  step,
  integer,
}: {
  label: string;
  hint: string;
  field: NumberField;
  s: HostSettings;
  set: <K extends keyof HostSettings>(k: K, v: HostSettings[K]) => void;
  step: number;
  integer?: boolean;
}) {
  const value = s.chat_defaults?.[field];
  return (
    <label>
      {label}
      <input
        type="number"
        step={step}
        placeholder="Model default"
        value={value ?? ""}
        onChange={(e) => {
          const raw = e.target.value.trim();
          const n = integer ? Math.round(Number(raw)) : Number(raw);
          set("chat_defaults", { ...s.chat_defaults, [field]: raw === "" || Number.isNaN(n) ? undefined : n });
        }}
      />
      <span className="muted small">{hint}</span>
    </label>
  );
}
