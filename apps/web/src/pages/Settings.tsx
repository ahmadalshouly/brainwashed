import { useEffect, useState } from "react";
import type { HostSettings } from "@brainwashed/api";
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
      {saved && !action.error && <Banner kind="ok">Saved. Model settings apply the next time a model loads.</Banner>}

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
