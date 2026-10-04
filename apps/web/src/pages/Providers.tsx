import { useMemo, useState } from "react";
import type { ProviderInfo } from "@brainwashed/api";
import { Badge, Banner, Card, PageHeader, useAction, useHost, useLoad } from "../ui";

/** Providers with an OpenAI-compatible chat API, and where to get a key. */
const PRESETS = [
  { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1", keys: "https://platform.openai.com/api-keys" },
  { id: "anthropic", name: "Anthropic", baseUrl: "https://api.anthropic.com/v1", keys: "https://console.anthropic.com/settings/keys" },
  { id: "gemini", name: "Google Gemini", baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", keys: "https://aistudio.google.com/apikey" },
  { id: "openrouter", name: "OpenRouter", baseUrl: "https://openrouter.ai/api/v1", keys: "https://openrouter.ai/keys" },
  { id: "groq", name: "Groq", baseUrl: "https://api.groq.com/openai/v1", keys: "https://console.groq.com/keys" },
  { id: "mistral", name: "Mistral", baseUrl: "https://api.mistral.ai/v1", keys: "https://console.mistral.ai/api-keys" },
  { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", keys: "https://platform.deepseek.com/api_keys" },
  { id: "xai", name: "xAI", baseUrl: "https://api.x.ai/v1", keys: "https://console.x.ai" },
  { id: "together", name: "Together AI", baseUrl: "https://api.together.xyz/v1", keys: "https://api.together.ai/settings/api-keys" },
  { id: "ollama", name: "Ollama", baseUrl: "http://localhost:11434/v1", keys: "" },
  { id: "custom", name: "Other", baseUrl: "", keys: "" },
] as const;

interface Form {
  editing: boolean;
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  keySet: boolean;
  models: string[];
  members: boolean;
}

const blank = (p: (typeof PRESETS)[number]): Form => ({
  editing: false,
  id: p.id === "custom" ? "" : p.id,
  name: p.id === "custom" ? "" : p.name,
  baseUrl: p.baseUrl,
  apiKey: "",
  keySet: false,
  models: [],
  members: true,
});

export function ProvidersPage() {
  const { remote } = useHost();
  const list = useLoad<ProviderInfo[]>(() => remote.providers(), [remote]);
  const [form, setForm] = useState<Form | null>(null);
  const [offered, setOffered] = useState<string[] | null>(null);
  const [filter, setFilter] = useState("");
  const [typed, setTyped] = useState("");
  const action = useAction();
  const check = useAction();

  const preset = PRESETS.find((p) => p.id === form?.id);
  const shown = useMemo(
    () => (offered ?? []).filter((m) => m.toLowerCase().includes(filter.toLowerCase())).slice(0, 200),
    [offered, filter],
  );

  function start(p: (typeof PRESETS)[number]) {
    setForm(blank(p));
    setOffered(null);
    setFilter("");
    check.setError(null);
    action.setError(null);
  }

  function edit(p: ProviderInfo) {
    setForm({ editing: true, id: p.id, name: p.name, baseUrl: p.baseUrl, apiKey: "", keySet: p.keySet, models: p.models, members: p.members });
    setOffered(null);
    setFilter("");
  }

  async function loadModels() {
    if (!form) return;
    const names = await check.run(() => remote.providerModels(form.baseUrl, form.apiKey || undefined, form.editing ? form.id : undefined));
    if (names) setOffered(names);
  }

  async function save() {
    if (!form) return;
    const done = await action.run(() =>
      remote.saveProvider({
        id: form.id,
        name: form.name,
        baseUrl: form.baseUrl,
        ...(form.apiKey ? { apiKey: form.apiKey } : {}),
        models: form.models,
        members: form.members,
      }),
    );
    if (done) {
      setForm(null);
      list.reload();
    }
  }

  async function remove(p: ProviderInfo) {
    if (!confirm(`Disconnect ${p.name}? Its API key is deleted from this computer.`)) return;
    await action.run(() => remote.deleteProvider(p.id));
    list.reload();
  }

  const toggle = (m: string) =>
    form && setForm({ ...form, models: form.models.includes(m) ? form.models.filter((x) => x !== m) : [...form.models, m] });

  return (
    <div className="page">
      <PageHeader
        title="Cloud models"
        subtitle="Use OpenAI, Anthropic, Gemini and others with your own API key, alongside or instead of models on this computer."
      />
      <Banner kind="info">
        Messages sent to a cloud model leave this computer and go to that provider. The chat shows this whenever a cloud model is picked. API keys stay on this
        computer and are never sent to devices.
      </Banner>
      {action.error && <Banner>{action.error}</Banner>}

      {(list.value?.length ?? 0) > 0 && (
        <Card title="Connected">
          <div className="provider-list">
            {list.value!.map((p) => (
              <div key={p.id} className="provider">
                <div className="provider-main">
                  <strong>{p.name}</strong>
                  <span className="muted small mono">{p.baseUrl}</span>
                  <div className="provider-models">
                    {p.models.length ? p.models.map((m) => <Badge key={m}>{m}</Badge>) : <span className="muted small">No models picked yet</span>}
                  </div>
                </div>
                <div className="provider-side">
                  <span className="muted small">{p.keySet ? `Key ${p.keyHint}` : "No key"}</span>
                  <span className="muted small">{p.members ? "Everyone can use it" : "Admins only"}</span>
                  <div className="actions">
                    <button onClick={() => edit(p)}>Edit</button>
                    <button className="danger-text" onClick={() => remove(p)}>
                      Disconnect
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {!form && (
        <Card title="Connect a provider">
          <div className="preset-grid">
            {PRESETS.map((p) => (
              <button key={p.id} className="preset" onClick={() => start(p)}>
                <strong>{p.name}</strong>
                <span className="muted small">{p.id === "custom" ? "Any OpenAI-compatible API" : p.id === "ollama" ? "On another computer" : new URL(p.baseUrl).hostname}</span>
              </button>
            ))}
          </div>
        </Card>
      )}

      {form && (
        <Card
          title={form.editing ? `Edit ${form.name}` : `Connect ${form.name || "a provider"}`}
          actions={<button onClick={() => setForm(null)}>Cancel</button>}
        >
          <div className="form">
            {(!preset || preset.id === "custom" || form.editing) && (
              <div className="form-row">
                <label>
                  Name
                  <input value={form.name} placeholder="My provider" onChange={(e) => setForm({ ...form, name: e.target.value })} />
                </label>
                <label>
                  Short id
                  <input
                    value={form.id}
                    disabled={form.editing}
                    placeholder="myprovider"
                    onChange={(e) => setForm({ ...form, id: e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, "") })}
                  />
                </label>
              </div>
            )}
            <label>
              API address
              <input className="mono" value={form.baseUrl} placeholder="https://api.example.com/v1" onChange={(e) => setForm({ ...form, baseUrl: e.target.value })} />
            </label>
            <label>
              API key
              <input
                type="password"
                autoComplete="off"
                value={form.apiKey}
                placeholder={form.keySet ? "Saved. Type a new key to replace it" : form.id === "ollama" ? "Not needed for Ollama" : "Paste your key"}
                onChange={(e) => setForm({ ...form, apiKey: e.target.value })}
              />
              {preset?.keys && (
                <span className="muted small">
                  Get one at{" "}
                  <a href={preset.keys} target="_blank" rel="noopener noreferrer">
                    {new URL(preset.keys).hostname}
                  </a>
                  .
                </span>
              )}
            </label>
            <div className="actions">
              <button disabled={check.busy || !form.baseUrl} onClick={loadModels}>
                {check.busy ? "Checking…" : offered ? "Reload models" : "Check the key and list models"}
              </button>
            </div>
            {check.error && <Banner>{check.error}</Banner>}

            <div className="field-block">
              <strong>Models to offer in the chat</strong>
              {offered && (
                <>
                  <input placeholder={`Filter ${offered.length} models`} value={filter} onChange={(e) => setFilter(e.target.value)} />
                  <div className="model-picks">
                    {shown.map((m) => (
                      <label key={m} className={`pick ${form.models.includes(m) ? "on" : ""}`}>
                        <input type="checkbox" checked={form.models.includes(m)} onChange={() => toggle(m)} />
                        {m}
                      </label>
                    ))}
                  </div>
                </>
              )}
              <div className="inline-add">
                <input
                  placeholder="Or type a model name, e.g. gpt-4o-mini"
                  value={typed}
                  onChange={(e) => setTyped(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && typed.trim()) {
                      e.preventDefault();
                      if (!form.models.includes(typed.trim())) setForm({ ...form, models: [...form.models, typed.trim()] });
                      setTyped("");
                    }
                  }}
                />
                <button
                  disabled={!typed.trim()}
                  onClick={() => {
                    if (!form.models.includes(typed.trim())) setForm({ ...form, models: [...form.models, typed.trim()] });
                    setTyped("");
                  }}
                >
                  Add
                </button>
              </div>
              {form.models.length > 0 && (
                <div className="provider-models">
                  {form.models.map((m) => (
                    <button key={m} className="chosen" onClick={() => toggle(m)} title="Remove">
                      {m} ×
                    </button>
                  ))}
                </div>
              )}
            </div>
            <label className="check">
              <input type="checkbox" checked={form.members} onChange={(e) => setForm({ ...form, members: e.target.checked })} />
              Members can use these models too (usage is billed to this key)
            </label>
            <div className="actions">
              <button className="primary" disabled={action.busy || !form.id || !form.baseUrl || form.models.length === 0} onClick={save}>
                {form.editing ? "Save" : "Connect"}
              </button>
            </div>
          </div>
        </Card>
      )}
    </div>
  );
}
