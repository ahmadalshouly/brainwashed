import { useCallback, useEffect, useState } from "react";
import QRCode from "qrcode";
import type { PairedDevice, PairingOffer, PhoneAccessStatus } from "@brainwashed/api";
import { engine, errorText, onGatewayEvent } from "../engine";

export function PhonesView() {
  const [status, setStatus] = useState<PhoneAccessStatus | null>(null);
  const [devices, setDevices] = useState<PairedDevice[]>([]);
  const [offer, setOffer] = useState<{ offer: PairingOffer; qr: string } | null>(null);
  const [justPaired, setJustPaired] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [relayDraft, setRelayDraft] = useState<string | null>(null);

  const refresh = useCallback(() => {
    engine.phoneStatus().then(setStatus);
    engine.devices().then(setDevices);
  }, []);

  useEffect(() => {
    refresh();
    return onGatewayEvent((e) => {
      if (e.type === "devicePaired") {
        setOffer(null);
        setJustPaired(e.device.name);
      }
      refresh();
    });
  }, [refresh]);

  // Pairing codes expire; drop the QR when this one does.
  useEffect(() => {
    if (!offer) return;
    const ms = offer.offer.expiresAt * 1000 - Date.now();
    const timer = setTimeout(() => setOffer(null), Math.max(ms, 0));
    return () => clearTimeout(timer);
  }, [offer]);

  const run = async <T,>(p: Promise<T>): Promise<T | undefined> => {
    setError(null);
    try {
      return await p;
    } catch (e) {
      setError(errorText(e));
    }
  };

  const toggle = async (enabled: boolean) => {
    const s = await run(engine.setPhoneAccess(enabled));
    if (s) setStatus(s);
    if (!enabled) setOffer(null);
  };

  // The relay's connection state changes on its own; keep it fresh while shown.
  useEffect(() => {
    if (!status?.relay) return;
    const timer = setInterval(() => engine.phoneStatus().then(setStatus), 3000);
    return () => clearInterval(timer);
  }, [!!status?.relay]);

  const saveRelay = async (url: string | null) => {
    const s = await run(engine.setRelayUrl(url));
    if (s) {
      setStatus(s);
      setRelayDraft(null);
    }
  };

  const showCode = async () => {
    setJustPaired(null);
    const o = await run(engine.createPairingOffer());
    if (!o) return;
    const qr = await QRCode.toDataURL(o.webUrl, { margin: 1, width: 240 });
    setOffer({ offer: o, qr });
  };

  return (
    <div className="phones">
      <div className="row header">
        <h2>Devices</h2>
        <label className="toggle">
          <input type="checkbox" checked={!!status?.running} onChange={(e) => toggle(e.target.checked)} />
          Allow phones and browsers on this network
        </label>
      </div>
      <p className="muted">
        Chat with this computer's models from any phone, tablet or computer on your network, in a web browser or the
        BrainWashed app. Only devices you pair here can connect, and messages are end-to-end encrypted.
      </p>
      {error && <div className="banner error">{error}</div>}
      {justPaired && <div className="banner ok">Paired with {justPaired}.</div>}

      {status?.running && (
        <section className="pairing">
          {status.addresses.length === 0 && !status.relay ? (
            <div className="banner error">This computer isn't on a local network, so phones can't reach it.</div>
          ) : offer ? (
            <div className="qr">
              <img src={offer.qr} alt="Pairing QR code" width={240} height={240} />
              <div>
                <strong>Scan with your phone's camera or the BrainWashed app</strong>
                <p className="muted">
                  The camera opens BrainWashed in the browser, no app needed. On another computer, open this link
                  instead:
                </p>
                <p className="small">
                  <code className="link">{offer.offer.webUrl}</code>
                </p>
                <p className="muted">
                  {status.relay
                    ? "Pair while the device is on the same Wi-Fi. The BrainWashed app then keeps working away from home through the relay; the web chat works on this network only."
                    : "The device must be on the same Wi-Fi."}{" "}
                  This code works once and expires in 10 minutes.
                </p>
                <button onClick={() => setOffer(null)}>Cancel</button>
              </div>
            </div>
          ) : (
            <button className="primary" onClick={showCode}>
              Pair a device
            </button>
          )}
          <p className="muted small">
            Listening on {status.addresses.join(", ")} port {status.port}
          </p>
        </section>
      )}

      <section>
        <h3>Away from home</h3>
        <p className="muted">
          To use this computer from anywhere, it keeps a connection open to a relay server. Messages pass through the
          relay still end-to-end encrypted, so it can't read them. Run your own relay (see docs/relay.md in the
          BrainWashed repository) or use one you trust.
        </p>
        {status?.relay && relayDraft === null ? (
          <div className="row">
            <div>
              <code>{status.relay.url}</code>
              <div className="muted small">
                {!status.running
                  ? "Turn on device access to connect."
                  : status.relay.connected
                    ? "Connected. Paired apps can reach this computer from anywhere."
                    : `Not connected${status.relay.error ? `: ${status.relay.error}` : ""}. Retrying.`}
              </div>
            </div>
            <div className="actions">
              <button onClick={() => setRelayDraft(status.relay!.url)}>Change</button>
              <button onClick={() => saveRelay(null)}>Turn off</button>
            </div>
          </div>
        ) : (
          <form
            className="row"
            onSubmit={(e) => {
              e.preventDefault();
              saveRelay(relayDraft?.trim() || null);
            }}
          >
            <input
              type="url"
              placeholder="https://relay.example.org"
              value={relayDraft ?? ""}
              onChange={(e) => setRelayDraft(e.target.value)}
            />
            <button className="primary" type="submit" disabled={!relayDraft?.trim()}>
              Save
            </button>
            {status?.relay && (
              <button type="button" onClick={() => setRelayDraft(null)}>
                Cancel
              </button>
            )}
          </form>
        )}
        {status?.relay && (
          <p className="muted small">Devices paired before you set the relay need to pair again to use it.</p>
        )}
      </section>

      <section>
        <h3>Paired devices</h3>
        {devices.length === 0 ? (
          <p className="hint">No devices yet.</p>
        ) : (
          <ul className="list">
            {devices.map((d) => (
              <li key={d.id}>
                <div>
                  <strong>{d.name}</strong>
                  <div className="muted small">
                    Paired {new Date(d.pairedAt * 1000).toLocaleDateString()}
                    {d.lastSeen ? ` · last used ${new Date(d.lastSeen * 1000).toLocaleString()}` : ""}
                  </div>
                </div>
                <div className="actions">
                  <button onClick={() => run(engine.removeDevice(d.id)).then(refresh)}>Remove</button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
