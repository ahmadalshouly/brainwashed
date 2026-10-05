import { useState } from "react";
import type { AccessStatus, ApiKey, DeviceRole, NewApiKey } from "@brainwashed/api";
import { ApiUsageView } from "./ApiUsage";
import { Badge, Banner, Card, CopyButton, PageHeader, timeAgo, useAction, useHost, useLoad } from "../ui";

interface Address {
  url: string;
  where: string;
  secure: boolean;
}

function addresses(a: AccessStatus | null | undefined): Address[] {
  const port = a?.port ?? 47860;
  const out: Address[] = [];
  if (a?.publicUrl)
    out.push({
      url: `${a.publicUrl.replace(/\/$/, "")}/v1`,
      where: "From anywhere",
      secure: true,
    });
  for (const ip of a?.addresses ?? []) {
    const host = ip.includes(":") ? `[${ip}]` : ip;
    out.push({
      url: `http://${host}:${port}/v1`,
      where: "On this network",
      secure: false,
    });
  }
  out.push({
    url: `http://localhost:${port}/v1`,
    where: "On this computer",
    secure: true,
  });
  return out;
}

function Snippet({ title, code }: { title: string; code: string }) {
  return (
    <div className="code">
      <div className="code-head">
        <span>{title}</span>
        <CopyButton text={code} />
      </div>
      <pre>
        <code>{code}</code>
      </pre>
    </div>
  );
}

export function ApiPage() {
  const { remote } = useHost();
  const access = useLoad<AccessStatus>(() => remote.access(), [remote], 10000);
  const keys = useLoad<ApiKey[]>(() => remote.apiKeys(), [remote], 30000);
  const action = useAction();
  const [name, setName] = useState("");
  const [role, setRole] = useState<DeviceRole>("member");
  const [created, setCreated] = useState<NewApiKey | null>(null);
  const [tab, setTab] = useState<"usage" | "setup" | null>(null);
  // Setup first until there's a key to use.
  const shown = tab ?? (keys.value?.length ? "usage" : keys.value ? "setup" : null);

  const list = addresses(access.value);
  const base = list[0]?.url ?? "http://localhost:47860/v1";
  const key = created?.secret ?? "YOUR_API_KEY";

  async function create() {
    const made = await action.run(() => remote.createApiKey(name.trim(), role));
    if (made) {
      setCreated(made);
      setName("");
      keys.reload();
    }
  }

  const curl = `curl ${base}/chat/completions \\
  -H "Authorization: Bearer ${key}" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "local", "messages": [{"role": "user", "content": "Hello!"}]}'`;
  const python = `from openai import OpenAI

client = OpenAI(base_url="${base}", api_key="${key}")
reply = client.chat.completions.create(
    model="local",
    messages=[{"role": "user", "content": "Hello!"}],
)
print(reply.choices[0].message.content)`;
  const js = `import OpenAI from "openai";

const client = new OpenAI({ baseURL: "${base}", apiKey: "${key}" });
const reply = await client.chat.completions.create({
  model: "local",
  messages: [{ role: "user", content: "Hello!" }],
});
console.log(reply.choices[0].message.content);`;

  return (
    <div className="page">
      <PageHeader
        title="API"
        subtitle="Use your models from scripts and other apps with any OpenAI-compatible client. Replies get the same skills and chat defaults as the chat."
      />
      {action.error && <Banner>{action.error}</Banner>}

      <div className="tabs" role="tablist">
        <button
          role="tab"
          aria-selected={shown === "usage"}
          className={shown === "usage" ? "on" : ""}
          onClick={() => setTab("usage")}
        >
          Usage
        </button>
        <button
          role="tab"
          aria-selected={shown === "setup"}
          className={shown === "setup" ? "on" : ""}
          onClick={() => setTab("setup")}
        >
          Keys and setup
        </button>
      </div>

      {shown === "usage" && <ApiUsageView keys={keys.value} />}
      {shown === "setup" && (
        <>
          <Card title="Address">
            <div className="api-addresses">
              {list.map((a) => (
                <div key={a.url} className="api-address">
                  <div className="grow">
                    <div className="muted small">
                      {a.where} {!a.secure && <Badge kind="warn">not encrypted</Badge>}
                    </div>
                    <div className="mono break">{a.url}</div>
                  </div>
                  <CopyButton text={a.url} />
                </div>
              ))}
            </div>
            <p className="muted small">
              Use it as the base URL in your OpenAI client.{" "}
              {list.some((a) => !a.secure) &&
                "The address on this network sends your key and messages without encryption, so use it only on networks you trust. "}
              {access.value?.publicUrl ? null : (
                <>
                  Turn on <strong>Remote access</strong> for an HTTPS address that works from anywhere.
                </>
              )}
            </p>
          </Card>

          <Card title="Keys">
            <div className="row wrap api-new-key">
              <input
                className="grow"
                placeholder="Name, e.g. Laptop scripts"
                value={name}
                aria-label="Key name"
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && create()}
              />
              <select value={role} onChange={(e) => setRole(e.target.value as DeviceRole)} aria-label="Access">
                <option value="member">Members' models</option>
                <option value="admin">All models</option>
              </select>
              <button className="primary" disabled={action.busy} onClick={create}>
                Create key
              </button>
            </div>
            {created && (
              <Banner kind="ok">
                <div className="row spread wrap">
                  <span>
                    Copy the key for <strong>{created.key.name}</strong> now. It won't be shown again.
                  </span>
                  <span className="row">
                    <span className="mono break">{created.secret}</span>
                    <CopyButton text={created.secret} />
                  </span>
                </div>
              </Banner>
            )}
            {keys.value?.length ? (
              <table className="table api-keys">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th>Can use</th>
                    <th>Key</th>
                    <th>Last used</th>
                    <th>Created</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {keys.value.map((k) => (
                    <tr key={k.id}>
                      <td>{k.name}</td>
                      <td className="muted">{k.role === "admin" ? "All models" : "Members' models"}</td>
                      <td className="mono muted">{k.hint}</td>
                      <td className="muted">{k.lastUsed ? timeAgo(k.lastUsed) : "never"}</td>
                      <td className="muted">{timeAgo(k.createdAt)}</td>
                      <td className="right">
                        <button
                          className="danger-text"
                          onClick={() => {
                            if (confirm(`Revoke ${k.name}? Apps using it will stop working.`))
                              action.run(async () => {
                                await remote.revokeApiKey(k.id);
                                if (created?.key.id === k.id) setCreated(null);
                                keys.reload();
                              });
                          }}
                        >
                          Revoke
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="muted">
                No keys yet. Create one for each app or script, so you can revoke it on its own. "Members' models" keys
                reach the model on this computer and the cloud models you share with members.
              </p>
            )}
          </Card>

          <Card title="Try it">
            <p className="muted small">
              Use <span className="mono">local</span> as the model for the one running on this computer, or any id from{" "}
              <span className="mono">{base}/models</span> for cloud models. Streaming, pictures as data URLs, and{" "}
              <span className="mono">reasoning_effort: "none"</span> to turn thinking off all work.
            </p>
            <Snippet title="curl" code={curl} />
            <Snippet title="Python" code={python} />
            <Snippet title="JavaScript" code={js} />
          </Card>
        </>
      )}
    </div>
  );
}
