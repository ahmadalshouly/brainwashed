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

  const showCode = async () => {
    setJustPaired(null);
    const o = await run(engine.createPairingOffer());
    if (!o) return;
    const qr = await QRCode.toDataURL(o.url, { margin: 1, width: 240 });
    setOffer({ offer: o, qr });
  };

  return (
    <div className="phones">
      <div className="row header">
        <h2>Phones</h2>
        <label className="toggle">
          <input type="checkbox" checked={!!status?.running} onChange={(e) => toggle(e.target.checked)} />
          Allow phones on this network
        </label>
      </div>
      <p className="muted">
        Pair the BrainWashed phone app to chat with this computer's models. Everything between the phone and this
        computer is end-to-end encrypted, and only phones you pair here can connect.
      </p>
      {error && <div className="banner error">{error}</div>}
      {justPaired && <div className="banner ok">Paired with {justPaired}.</div>}

      {status?.running && (
        <section className="pairing">
          {status.addresses.length === 0 ? (
            <div className="banner error">This computer isn't on a local network, so phones can't reach it.</div>
          ) : offer ? (
            <div className="qr">
              <img src={offer.qr} alt="Pairing QR code" width={240} height={240} />
              <div>
                <strong>Scan with the BrainWashed app</strong>
                <p className="muted">
                  Open the app, tap Pair a computer, and point the camera here. The phone must be on the same Wi-Fi.
                  This code works once and expires in 10 minutes.
                </p>
                <button onClick={() => setOffer(null)}>Cancel</button>
              </div>
            </div>
          ) : (
            <button className="primary" onClick={showCode}>
              Pair a phone
            </button>
          )}
          <p className="muted small">
            Listening on {status.addresses.join(", ")} port {status.port}
          </p>
        </section>
      )}

      <section>
        <h3>Paired phones</h3>
        {devices.length === 0 ? (
          <p className="hint">No phones yet.</p>
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
