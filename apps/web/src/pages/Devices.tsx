import { useEffect, useState } from "react";
import type { DeviceRole, PairedDevice, PairingOffer } from "@brainwashed/api";
import { Badge, Banner, Card, CopyButton, PageHeader, QrCode, timeAgo, useAction, useHost, useLoad } from "../ui";

export function DevicesPage() {
  const { remote } = useHost();
  const devices = useLoad<PairedDevice[]>(() => remote.devices(), [remote], 10000);
  const action = useAction();
  const [inviteRole, setInviteRole] = useState<DeviceRole>("member");
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [left, setLeft] = useState(0);

  useEffect(() => {
    if (!offer) return;
    const tick = () => {
      const s = Math.max(0, Math.round(offer.expiresAt - Date.now() / 1000));
      setLeft(s);
      if (s === 0) setOffer(null);
    };
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [offer]);

  // A device that pairs shows up in the list; close the code then.
  const count = devices.value?.length;
  const reloadDevices = devices.reload;
  useEffect(() => {
    if (!offer) return;
    const timer = setInterval(reloadDevices, 2000);
    return () => clearInterval(timer);
  }, [offer, reloadDevices]);
  const [countAtInvite, setCountAtInvite] = useState<number | undefined>(undefined);
  useEffect(() => {
    if (offer && countAtInvite !== undefined && count !== undefined && count > countAtInvite) setOffer(null);
  }, [count, countAtInvite, offer]);

  async function invite() {
    const o = await action.run(() => remote.createPairingOffer(inviteRole));
    if (o) {
      setCountAtInvite(devices.value?.length);
      setOffer(o);
    }
  }

  return (
    <div className="page">
      <PageHeader
        title="Devices"
        subtitle="Phones, tablets and browsers that can use this computer. Admins manage everything; members can only chat."
      />
      {action.error && <Banner>{action.error}</Banner>}

      <Card
        title="Add a device"
        actions={
          !offer && (
            <>
              <select value={inviteRole} onChange={(e) => setInviteRole(e.target.value as DeviceRole)} aria-label="Role">
                <option value="member">As a member (chat only)</option>
                <option value="admin">As an admin</option>
              </select>
              <button className="primary" disabled={action.busy} onClick={invite}>
                Show code
              </button>
            </>
          )
        }
      >
        {offer ? (
          <div className="invite">
            {offer.qr && <QrCode rows={offer.qr} />}
            <div className="grow">
              <p>
                Scan with the phone's camera to connect it as {offer.role === "admin" ? "an admin" : "a member"}.{" "}
                {offer.publicUrl
                  ? "It works from anywhere."
                  : "The phone must be on the same network, because remote access is off or still connecting."}
              </p>
              <p className="muted small">
                Or open this link on the device. It works once and expires in {Math.floor(left / 60)}:
                {String(left % 60).padStart(2, "0")}.
              </p>
              <div className="row">
                <CopyButton text={offer.webUrl} label="Copy link" />
                <button onClick={() => setOffer(null)}>Done</button>
              </div>
              <p className="muted small">
                The BrainWashed phone app scans the same code.
              </p>
            </div>
          </div>
        ) : (
          <p className="muted">
            Create a one-time code, then scan it on the device. Each device gets its own encryption keys, and you can remove
            it here at any time.
          </p>
        )}
      </Card>

      <Card title="Connected devices">
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Role</th>
              <th>Last seen</th>
              <th>Added</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {devices.value?.map((d) => (
              <tr key={d.id}>
                <td>
                  <button
                    className="ghost link"
                    title="Rename"
                    onClick={() => {
                      const name = prompt("Name this device", d.name);
                      if (name?.trim()) action.run(async () => (await remote.renameDevice(d.id, name.trim()), devices.reload()));
                    }}
                  >
                    {d.name}
                  </button>{" "}
                  {d.current && <Badge kind="ok">this browser</Badge>}
                </td>
                <td>
                  <select
                    value={d.role}
                    aria-label={`Role of ${d.name}`}
                    onChange={(e) =>
                      action.run(async () => (await remote.setDeviceRole(d.id, e.target.value as DeviceRole), devices.reload()))
                    }
                  >
                    <option value="admin">Admin</option>
                    <option value="member">Member</option>
                  </select>
                </td>
                <td className="muted">{timeAgo(d.lastSeen)}</td>
                <td className="muted">{timeAgo(d.pairedAt)}</td>
                <td className="right">
                  <button
                    className="danger-text"
                    onClick={() => {
                      const msg = d.current
                        ? "Remove this browser? You'll need a new code to connect it again."
                        : `Remove ${d.name}? It won't be able to connect any more.`;
                      if (confirm(msg)) action.run(async () => (await remote.removeDevice(d.id), devices.reload()));
                    }}
                  >
                    Remove
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}
