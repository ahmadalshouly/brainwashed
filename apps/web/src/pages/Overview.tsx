import type { AccessStatus, Hardware, InstalledModel, PairedDevice, UpdateInfo } from "@brainwashed/api";
import { Badge, Banner, Card, CopyButton, formatBytes, PageHeader, stateText, useHost, useLoad } from "../ui";

export function OverviewPage() {
  const { remote, state, go } = useHost();
  const access = useLoad<AccessStatus>(() => remote.access(), [remote], 5000);
  const models = useLoad<InstalledModel[]>(() => remote.models(), [remote]);
  const devices = useLoad<PairedDevice[]>(() => remote.devices(), [remote], 15000);
  const hw = useLoad<Hardware>(() => remote.hardware(), [remote]);
  const update = useLoad<UpdateInfo | null>(() => remote.checkForUpdate(), [remote]);

  const modelName = (id: string) => models.value?.find((m) => m.id === id)?.name ?? id;
  const a = access.value;
  const tunnel = a?.tunnel;

  return (
    <div className="page">
      <PageHeader title="Overview" subtitle="Everything running on this computer at a glance." />
      {update.value && (
        <Banner kind="info">
          BrainWashed {update.value.version} is available.{" "}
          <a href={update.value.url} target="_blank" rel="noopener noreferrer">
            See what's new
          </a>{" "}
          and update by running the install command again.
        </Banner>
      )}
      <div className="grid">
        <Card title="Model" actions={<button onClick={() => go("models")}>Manage</button>}>
          <p className="big">{stateText(state, modelName)}</p>
          <p className="muted small">{models.value ? `${models.value.length} installed` : "…"}</p>
        </Card>

        <Card title="Remote access" actions={<button onClick={() => go("remote")}>Configure</button>}>
          {a?.publicUrl ? (
            <>
              <p className="big mono ellipsis">{a.publicUrl}</p>
              <div className="row">
                <Badge kind="ok">Online</Badge>
                <CopyButton text={a.publicUrl} label="Copy address" />
              </div>
            </>
          ) : tunnel ? (
            <>
              <p className="big">Connecting…</p>
              <p className="muted small">{tunnel.error ?? "Starting the tunnel"}</p>
            </>
          ) : a?.relay ? (
            <>
              <p className="big">Through your relay</p>
              <p className="muted small">{a.relay.connected ? "Connected" : (a.relay.error ?? "Not connected")}</p>
            </>
          ) : (
            <>
              <p className="big">Off</p>
              <p className="muted small">Only devices on your network can connect.</p>
            </>
          )}
        </Card>

        <Card title="Devices" actions={<button onClick={() => go("devices")}>Add a device</button>}>
          <p className="big">{devices.value?.length ?? "…"}</p>
          <p className="muted small">
            {devices.value
              ? `${devices.value.filter((d) => d.role === "admin").length} admins, ${devices.value.filter((d) => d.role === "member").length} members`
              : ""}
          </p>
        </Card>

        <Card title="This computer">
          {hw.value ? (
            <>
              <p className="big">{formatBytes(hw.value.total_memory_bytes)} memory</p>
              <p className="muted small">
                {hw.value.cpu_cores} CPU cores · {hw.value.os} {hw.value.arch}
              </p>
            </>
          ) : (
            <p className="muted">…</p>
          )}
        </Card>
      </div>

      {a && (
        <Card title="Addresses">
          <table className="table">
            <tbody>
              <tr>
                <td>This computer</td>
                <td className="mono">http://localhost:{a.port}</td>
              </tr>
              {a.addresses.slice(0, 1).map((ip) => (
                <tr key={ip}>
                  <td>Local network</td>
                  <td className="mono">
                    http://{ip}:{a.port}
                  </td>
                </tr>
              ))}
              <tr>
                <td>Anywhere</td>
                <td className="mono">{a.publicUrl ?? "—"}</td>
              </tr>
              {a.relay && (
                <tr>
                  <td>Relay</td>
                  <td className="mono">{a.relay.url}</td>
                </tr>
              )}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  );
}
