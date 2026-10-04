import { useEffect, useState } from "react";
import type { AccessStatus, HostSettings, RemoteAccess } from "@brainwashed/api";
import { Badge, Banner, Card, CopyButton, PageHeader, useAction, useHost, useLoad } from "../ui";

type Mode = RemoteAccess | "url";

const OPTIONS: { id: Mode; title: string; body: string }[] = [
  {
    id: "quick",
    title: "Free tunnel (recommended to start)",
    body: "A free Cloudflare address with no account or router setup. The address changes each time BrainWashed starts, so devices need a new code after a restart.",
  },
  {
    id: "cloudflare",
    title: "Your own domain with Cloudflare",
    body: "A stable address like https://ai.yourcompany.com. Create a tunnel in the Cloudflare dashboard (Zero Trust → Networks → Tunnels), point its public hostname at http://localhost:" +
      "PORT, and paste its token here.",
  },
  {
    id: "url",
    title: "An address you set up yourself",
    body: "Tailscale Funnel, ngrok, or your own reverse proxy that forwards to this computer. Enter its https address.",
  },
  { id: "off", title: "Off", body: "Only devices on the same network can connect." },
];

export function RemotePage() {
  const { remote } = useHost();
  const access = useLoad<AccessStatus>(() => remote.access(), [remote], 3000);
  const settings = useLoad<HostSettings>(() => remote.settings(), [remote]);
  const action = useAction();
  const [mode, setMode] = useState<Mode>("quick");
  const [token, setToken] = useState("");
  const [url, setUrl] = useState("");
  const [relay, setRelay] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    const s = settings.value;
    if (!s) return;
    setMode(s.remote_access === "off" && s.public_url ? "url" : s.remote_access);
    setUrl(s.public_url ?? "");
    setRelay(s.relay_url ?? "");
  }, [settings.value]);

  async function save() {
    setSaved(false);
    const patch: Partial<HostSettings> = {
      remote_access: mode === "url" ? "off" : mode,
      public_url: mode === "cloudflare" || mode === "url" ? url.trim() || null : null,
      relay_url: relay.trim() || null,
    };
    if (mode === "cloudflare" && token.trim()) patch.tunnel_token = token.trim();
    const next = await action.run(() => remote.updateSettings(patch));
    if (next) {
      settings.reload();
      access.reload();
      setToken("");
      setSaved(true);
    }
  }

  const a = access.value;
  const port = a?.port ?? 47860;

  return (
    <div className="page">
      <PageHeader
        title="Remote access"
        subtitle="Reach this computer from anywhere, not just your Wi-Fi. Chats stay end-to-end encrypted between each device and this computer whichever way you pick."
      />
      {action.error && <Banner>{action.error}</Banner>}
      {saved && !action.error && <Banner kind="ok">Saved and applied.</Banner>}

      <Card title="Status">
        {a?.publicUrl ? (
          <div className="row spread wrap">
            <div>
              <Badge kind="ok">Online</Badge> <span className="mono">{a.publicUrl}</span>
            </div>
            <div className="row">
              <CopyButton text={a.publicUrl} />
              <a className="button" href={a.publicUrl} target="_blank" rel="noopener noreferrer">
                Open
              </a>
            </div>
          </div>
        ) : a?.tunnel ? (
          <p>
            <Badge kind="warn">Connecting</Badge> {a.tunnel.error ?? "Starting the tunnel…"}
          </p>
        ) : (
          <p>
            <Badge>Off</Badge> Only devices on your network can connect.
          </p>
        )}
        {a?.relay && (
          <p className="small">
            Relay <span className="mono">{a.relay.url}</span>:{" "}
            {a.relay.connected ? <Badge kind="ok">connected</Badge> : <Badge kind="warn">{a.relay.error ?? "not connected"}</Badge>}
          </p>
        )}
        <p className="muted small">
          New devices get the remote address in their code. To connect a device, go to Devices and click Show code.
        </p>
      </Card>

      <Card title="How devices reach this computer">
        <div className="options">
          {OPTIONS.map((o) => (
            <label key={o.id} className={`option ${mode === o.id ? "on" : ""}`}>
              <input type="radio" name="mode" checked={mode === o.id} onChange={() => setMode(o.id)} />
              <div>
                <strong>{o.title}</strong>
                <p className="muted small">{o.body.replace("PORT", String(port))}</p>
                {o.id === "cloudflare" && mode === "cloudflare" && (
                  <div className="fields">
                    <label>
                      Tunnel token {settings.value?.tunnel_token_set && <Badge kind="ok">saved</Badge>}
                      <input
                        type="password"
                        autoComplete="off"
                        placeholder={settings.value?.tunnel_token_set ? "Leave empty to keep the saved token" : "eyJhIjoi…"}
                        value={token}
                        onChange={(e) => setToken(e.target.value)}
                      />
                    </label>
                    <label>
                      Public hostname
                      <input placeholder="https://ai.example.com" value={url} onChange={(e) => setUrl(e.target.value)} />
                    </label>
                  </div>
                )}
                {o.id === "url" && mode === "url" && (
                  <div className="fields">
                    <label>
                      Address
                      <input placeholder="https://my-pc.tailnet.ts.net" value={url} onChange={(e) => setUrl(e.target.value)} />
                    </label>
                  </div>
                )}
              </div>
            </label>
          ))}
        </div>
        <details className="advanced">
          <summary>Self-hosted relay (advanced)</summary>
          <p className="muted small">
            Run the BrainWashed relay on your own server and the phone app connects through it. It only ever sees encrypted
            traffic. See docs/relay.md.
          </p>
          <input placeholder="https://relay.example.org" value={relay} onChange={(e) => setRelay(e.target.value)} />
        </details>
        <div className="row end">
          <button className="primary" disabled={action.busy} onClick={save}>
            Save and apply
          </button>
        </div>
      </Card>
    </div>
  );
}
