import { useEffect, useState } from "react";
import type { UpdateInfo } from "@brainwashed/api";
import { engine } from "../engine";

/** Version, update notice and the update-check switch, at the bottom of the sidebar. */
export function About() {
  const [version, setVersion] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checks, setChecks] = useState(true);

  useEffect(() => {
    engine.info().then((i) => setVersion(i.version));
    engine.updateChecksEnabled().then(setChecks);
  }, []);

  useEffect(() => {
    if (checks) engine.checkForUpdate().then(setUpdate);
    else setUpdate(null);
  }, [checks]);

  const toggle = async (enabled: boolean) => {
    await engine.setUpdateChecks(enabled);
    setChecks(enabled);
  };

  return (
    <div className="about">
      {update && (
        <button className="update" onClick={() => engine.openReleasePage(update.url)}>
          Version {update.version} is available
        </button>
      )}
      <div className="muted small">BrainWashed {version && version !== "0.0.0" ? `v${version}` : "(development build)"}</div>
      <label className="small muted">
        <input type="checkbox" checked={checks} onChange={(e) => toggle(e.target.checked)} /> Check for updates
      </label>
    </div>
  );
}
