import { useState } from "react";
import type { CatalogItem, DownloadStatus, Hardware, InstalledModel } from "@brainwashed/api";
import { Badge, Banner, Card, formatBytes, PageHeader, Progress, useAction, useHost, useLoad } from "../ui";

export function ModelsPage() {
  const { remote, state, refreshState } = useHost();
  const models = useLoad<InstalledModel[]>(() => remote.models(), [remote], 10000);
  const catalog = useLoad<CatalogItem[]>(() => remote.catalog(), [remote], 30000);
  const hw = useLoad<Hardware>(() => remote.hardware(), [remote]);
  const [polling, setPolling] = useState(true);
  const downloads = useLoad<DownloadStatus[]>(
    async () => {
      const d = await remote.downloads();
      const active = d.some((x) => !x.finished);
      setPolling(active);
      if (d.some((x) => x.finished)) models.reload();
      return d;
    },
    [remote],
    polling ? 1500 : 10000,
  );
  const action = useAction();
  const [repo, setRepo] = useState("");
  const [quant, setQuant] = useState("");

  const current = state && "model" in state ? state.model : null;
  const downloading = (r: string) => downloads.value?.find((d) => d.repo === r && !d.finished);

  const download = (r: string, q?: string) =>
    action.run(async () => {
      await remote.downloadModel(r, q || undefined);
      setPolling(true);
      downloads.reload();
    });

  return (
    <div className="page">
      <PageHeader
        title="Models"
        subtitle={
          hw.value
            ? `This computer has ${formatBytes(hw.value.total_memory_bytes)} of memory. Models marked "fits" run comfortably.`
            : "Download open source models and choose which one runs."
        }
      />
      {action.error && <Banner>{action.error}</Banner>}

      <Card
        title="Running now"
        actions={
          current && (
            <button
              disabled={action.busy}
              onClick={() => action.run(async () => (await remote.unloadModel(), refreshState()))}
            >
              Stop model
            </button>
          )
        }
      >
        <p>
          {state?.state === "ready" && <>{models.value?.find((m) => m.id === current)?.name ?? current}</>}
          {state?.state === "loading" && <>Loading {models.value?.find((m) => m.id === current)?.name ?? current}…</>}
          {state?.state === "installingRuntime" && (
            <>
              Downloading llama.cpp… <Progress done={state.done} total={state.total} />
            </>
          )}
          {state?.state === "error" && <span className="danger">{state.message}</span>}
          {state?.state === "idle" && <span className="muted">No model is running. Load one below.</span>}
        </p>
      </Card>

      {(downloads.value?.length ?? 0) > 0 && (
        <Card title="Downloads">
          {downloads.value!.map((d) => (
            <div key={d.repo} className="download">
              <div className="row spread">
                <span className="mono">{d.repo}</span>
                <span className="muted small">
                  {d.error
                    ? "Failed"
                    : d.finished
                      ? "Done"
                      : `${formatBytes(d.done)}${d.total ? ` of ${formatBytes(d.total)}` : ""}`}
                </span>
              </div>
              {!d.finished && <Progress done={d.done} total={d.total} />}
              {d.error && <p className="danger small">{d.error}</p>}
            </div>
          ))}
        </Card>
      )}

      <Card title="Installed">
        {models.value?.length === 0 && <p className="muted">No models yet. Download one below.</p>}
        <div className="list">
          {models.value?.map((m) => {
            const speedups = models.value!.filter((d) => d.draft);
            return (
              <div key={m.id} className="list-row wrap">
                <div className="grow">
                  <strong>{m.name}</strong> {m.mmproj && <Badge>sees pictures</Badge>}
                  {m.draft && <Badge>speed-up</Badge>}
                  <div className="muted small">
                    {formatBytes(m.size)}
                    {m.repo ? ` · ${m.repo}` : ""}
                  </div>
                  {m.draft && (
                    <div className="muted small">
                      Makes the model it was trained for answer faster. It can't run on its own: pick it as the speed-up
                      of that model.
                    </div>
                  )}
                </div>
                {!m.draft && speedups.length > 0 && (
                  <select
                    aria-label={`Speed-up for ${m.name}`}
                    value={m.speedup ?? ""}
                    disabled={action.busy}
                    onChange={(e) =>
                      action.run(async () => {
                        await remote.setModelSpeedup(m.id, e.target.value || null);
                        models.reload();
                        refreshState();
                      })
                    }
                  >
                    <option value="">No speed-up</option>
                    {speedups.map((d) => (
                      <option key={d.id} value={d.id}>
                        Speed up with {d.name}
                      </option>
                    ))}
                  </select>
                )}
                {m.draft ? null : m.id === current ? (
                  <Badge kind="ok">{state?.state === "ready" ? "Running" : "Loading"}</Badge>
                ) : (
                  <button
                    className="primary"
                    disabled={action.busy || state?.state === "loading"}
                    onClick={() => action.run(async () => (await remote.loadModel(m.id), refreshState()))}
                  >
                    Use
                  </button>
                )}
                <button
                  className="danger-text"
                  disabled={action.busy}
                  onClick={() => {
                    if (confirm(`Delete ${m.name}? This frees ${formatBytes(m.size)}.`))
                      action.run(async () => (await remote.deleteModel(m.id), models.reload(), refreshState()));
                  }}
                >
                  Delete
                </button>
              </div>
            );
          })}
        </div>
      </Card>

      <Card title="Suggested">
        <div className="list">
          {catalog.value?.map((c) => {
            const d = downloading(c.repo);
            return (
              <div key={c.repo} className="list-row">
                <div className="grow">
                  <strong>{c.name}</strong>{" "}
                  {c.fits ? <Badge kind="ok">fits</Badge> : <Badge kind="warn">needs more memory</Badge>}
                  {c.vision && <Badge>sees pictures</Badge>}
                  <div className="muted small">
                    {c.description} · {Math.round(c.params_b * 10) / 10}B parameters · {c.license}
                  </div>
                </div>
                {c.installed ? (
                  <Badge>Installed</Badge>
                ) : d ? (
                  <span className="muted small">Downloading…</span>
                ) : (
                  <button disabled={action.busy} onClick={() => download(c.repo)}>
                    Download
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </Card>

      <Card title="Any model from Hugging Face">
        <p className="muted small">
          Paste a GGUF repository, like <span className="mono">Qwen/Qwen3-4B-GGUF</span>. BrainWashed picks a good
          quantization, or name one (for example <span className="mono">Q4_K_M</span>).
        </p>
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            if (repo.trim()) download(repo.trim(), quant.trim()).then(() => setRepo(""));
          }}
        >
          <input
            className="grow"
            placeholder="owner/model-GGUF"
            value={repo}
            onChange={(e) => setRepo(e.target.value)}
          />
          <input
            style={{ width: 120 }}
            placeholder="Quantization"
            value={quant}
            onChange={(e) => setQuant(e.target.value)}
          />
          <button className="primary" disabled={!repo.trim() || action.busy}>
            Download
          </button>
        </form>
      </Card>
    </div>
  );
}
