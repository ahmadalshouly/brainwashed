import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { HostInfo } from "@brainwashed/api";

export function App() {
  const [info, setInfo] = useState<HostInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<HostInfo>("host_info").then(setInfo, (e) => setError(String(e)));
  }, []);

  return (
    <main>
      <h1>BrainWashed</h1>
      <p className="tagline">Your laptop, your AI.</p>
      {error && <p className="error">{error}</p>}
      {info && (
        <dl>
          <dt>Host</dt>
          <dd>{info.name}</dd>
          <dt>Version</dt>
          <dd>{info.version}</dd>
          <dt>Model</dt>
          <dd>{info.model ?? "No model loaded yet"}</dd>
        </dl>
      )}
    </main>
  );
}
