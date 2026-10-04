import type { AuditEntry } from "@brainwashed/api";
import { Banner, Card, PageHeader, useHost, useLoad } from "../ui";

const LABELS: Record<string, string> = {
  pair: "Device connected",
  loadModel: "Loaded a model",
  unloadModel: "Stopped the model",
  downloadModel: "Downloaded a model",
  deleteModel: "Deleted a model",
  setSkillEnabled: "Turned a skill on or off",
  saveSkill: "Saved a skill",
  deleteSkill: "Deleted a skill",
  updateSettings: "Changed settings",
  createPairingOffer: "Created a device code",
  removeDevice: "Removed a device",
  setDeviceRole: "Changed a device's role",
  renameDevice: "Renamed a device",
};

export function ActivityPage() {
  const { remote } = useHost();
  const log = useLoad<AuditEntry[]>(() => remote.auditLog(200), [remote], 10000);
  return (
    <div className="page">
      <PageHeader
        title="Activity"
        subtitle="Who changed what on this computer. Chats aren't recorded. The log is kept on this computer in the gateway folder."
      />
      {log.error && <Banner>{log.error}</Banner>}
      <Card>
        {log.value?.length === 0 && <p className="muted">Nothing yet.</p>}
        <table className="table">
          <thead>
            <tr>
              <th>When</th>
              <th>Who</th>
              <th>What</th>
              <th>Details</th>
            </tr>
          </thead>
          <tbody>
            {log.value?.map((e, i) => (
              <tr key={i}>
                <td className="muted nowrap">{new Date(e.at * 1000).toLocaleString()}</td>
                <td>{e.deviceName ?? "This computer's terminal"}</td>
                <td>{LABELS[e.action] ?? e.action}</td>
                <td>
                  <span className="mono small">{e.target ?? ""}</span>
                  {e.error && <span className="danger small"> Failed: {e.error}</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}
