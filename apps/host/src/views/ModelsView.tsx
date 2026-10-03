import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { CatalogItem, EngineState, InstalledModel } from "@brainwashed/api";
import { engine, errorText, formatBytes, onEngineEvent } from "../engine";

type Progress = Record<string, { done: number; total: number | null }>;

export function ModelsView({ state }: { state: EngineState }) {
  const [catalog, setCatalog] = useState<CatalogItem[]>([]);
  const [models, setModels] = useState<InstalledModel[]>([]);
  const [progress, setProgress] = useState<Progress>({});
  const [customRepo, setCustomRepo] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    engine.catalog().then(setCatalog);
    engine.models().then(setModels);
  }, []);

  useEffect(() => {
    refresh();
    return onEngineEvent((e) => {
      if (e.type === "downloadProgress") {
        setProgress((p) => ({ ...p, [e.repo]: { done: e.done, total: e.total } }));
      } else if (e.type === "downloadFinished" || e.type === "downloadFailed") {
        setProgress(({ [e.repo]: _, ...rest }) => rest);
        if (e.type === "downloadFailed") setError(e.error);
      } else if (e.type === "modelsChanged") {
        refresh();
      }
    });
  }, [refresh]);

  const run = (p: Promise<unknown>) => {
    setError(null);
    p.catch((e) => setError(errorText(e)));
  };

  const download = (repo: string) => {
    setProgress((p) => ({ ...p, [repo]: { done: 0, total: null } }));
    run(engine.download(repo));
  };

  const importFile = async () => {
    const path = await open({ multiple: false, filters: [{ name: "GGUF model", extensions: ["gguf"] }] });
    if (typeof path === "string") run(engine.importModel(path));
  };

  const active = state.state === "ready" || state.state === "loading" ? state.model : null;
  const busy = state.state === "loading" || state.state === "installingRuntime";

  return (
    <div className="models">
      {error && <div className="banner error">{error}</div>}
      {state.state === "error" && <div className="banner error">{state.message}</div>}

      <section>
        <h2>On this computer</h2>
        {models.length === 0 && <p className="hint">No models yet. Pick one below.</p>}
        <ul className="list">
          {models.map((m) => (
            <li key={m.id}>
              <div>
                <strong>{m.name}</strong>
                <span className="muted"> {formatBytes(m.size)}{m.repo ? ` · ${m.repo}` : ""}</span>
              </div>
              <div className="actions">
                {active === m.id && state.state === "ready" ? (
                  <button onClick={() => run(engine.unload())}>Stop</button>
                ) : (
                  <button className="primary" disabled={busy} onClick={() => run(engine.load(m.id))}>
                    {active === m.id ? "Loading…" : "Load"}
                  </button>
                )}
                <button disabled={busy && active === m.id} onClick={() => run(engine.deleteModel(m.id))}>
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ul>
        <button onClick={importFile}>Add a GGUF file from disk…</button>
      </section>

      <section>
        <h2>Get a model</h2>
        <ul className="list">
          {catalog.map((c) => (
            <li key={c.repo}>
              <div>
                <strong>{c.name}</strong>
                {!c.fits && <span className="tag">Needs more memory</span>}
                <div className="muted">{c.description} · {c.license}</div>
              </div>
              <div className="actions">
                <DownloadButton
                  installed={c.installed}
                  progress={progress[c.repo]}
                  onClick={() => download(c.repo)}
                />
              </div>
            </li>
          ))}
        </ul>
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            if (customRepo.trim()) download(customRepo.trim());
          }}
        >
          <input
            value={customRepo}
            onChange={(e) => setCustomRepo(e.target.value)}
            placeholder="Any Hugging Face GGUF repo, e.g. bartowski/Phi-4-mini-instruct-GGUF"
          />
          <DownloadButton progress={progress[customRepo.trim()]} submit />
        </form>
      </section>
    </div>
  );
}

function DownloadButton(props: {
  installed?: boolean;
  progress?: { done: number; total: number | null };
  onClick?: () => void;
  submit?: boolean;
}) {
  if (props.progress) {
    const { done, total } = props.progress;
    return (
      <span className="progress">
        {total ? `${Math.round((done / total) * 100)}% of ${formatBytes(total)}` : formatBytes(done)}
      </span>
    );
  }
  if (props.installed) return <span className="muted">Installed</span>;
  return (
    <button type={props.submit ? "submit" : "button"} onClick={props.onClick}>
      Download
    </button>
  );
}
