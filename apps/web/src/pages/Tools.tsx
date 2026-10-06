import { useState } from "react";
import type { McpServer, McpServerInfo, McpTransport } from "@brainwashed/api";
import { Badge, Banner, Card, PageHeader, useAction, useHost, useLoad } from "../ui";

/** Well-known servers, to fill the form in one click. */
const EXAMPLES: { id: string; name: string; about: string; transport: McpTransport }[] = [
  {
    id: "files",
    name: "Files",
    about: "Read and write files in a folder you choose",
    transport: { type: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/folder"], env: {} },
  },
  {
    id: "fetch",
    name: "Fetch",
    about: "Read web pages",
    transport: { type: "stdio", command: "uvx", args: ["mcp-server-fetch"], env: {} },
  },
  {
    id: "memory",
    name: "Memory",
    about: "Remember facts across chats",
    transport: { type: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-memory"], env: {} },
  },
  {
    id: "time",
    name: "Time",
    about: "Current time and time zones",
    transport: { type: "stdio", command: "uvx", args: ["mcp-server-time"], env: {} },
  },
];

interface Pair {
  key: string;
  value: string;
}

interface Form {
  editing: boolean;
  id: string;
  name: string;
  enabled: boolean;
  members: boolean;
  type: "stdio" | "http";
  command: string;
  /** One per line, so paths with spaces stay whole. */
  args: string;
  env: Pair[];
  cwd: string;
  url: string;
  headers: Pair[];
}

const pairs = (r: Record<string, string>): Pair[] => Object.entries(r).map(([key, value]) => ({ key, value }));
const record = (p: Pair[]) => Object.fromEntries(p.filter((x) => x.key.trim()).map((x) => [x.key.trim(), x.value]));
const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32);

function formOf(s: McpServer, editing: boolean): Form {
  const t = s.transport;
  return {
    editing,
    id: s.id,
    name: s.name,
    enabled: s.enabled,
    members: s.members,
    type: t.type,
    command: t.type === "stdio" ? t.command : "",
    args: t.type === "stdio" ? t.args.join("\n") : "",
    env: t.type === "stdio" ? pairs(t.env) : [],
    cwd: t.type === "stdio" ? (t.cwd ?? "") : "",
    url: t.type === "http" ? t.url : "",
    headers: t.type === "http" ? pairs(t.headers) : [],
  };
}

function serverOf(f: Form): McpServer {
  const transport: McpTransport =
    f.type === "stdio"
      ? {
          type: "stdio",
          command: f.command.trim(),
          args: f.args.split("\n").map((a) => a.trim()).filter(Boolean),
          env: record(f.env),
          ...(f.cwd.trim() ? { cwd: f.cwd.trim() } : {}),
        }
      : { type: "http", url: f.url.trim(), headers: record(f.headers) };
  return { id: f.id, name: f.name.trim() || f.id, enabled: f.enabled, members: f.members, transport };
}

const blank = (type: "stdio" | "http"): Form =>
  formOf({ id: "", name: "", enabled: true, members: false, transport: type === "stdio" ? { type, command: "", args: [], env: {} } : { type, url: "", headers: {} } }, false);

/**
 * Servers from a pasted config: the `mcpServers` object Claude Desktop,
 * Cursor and most READMEs use, or one server on its own.
 */
export function parseConfig(text: string): McpServer[] {
  const json = JSON.parse(text);
  if (!json || typeof json !== "object") throw new Error("That isn't a server config.");
  const entries: [string, Record<string, unknown>][] =
    json.mcpServers && typeof json.mcpServers === "object"
      ? Object.entries(json.mcpServers)
      : json.servers && typeof json.servers === "object"
        ? Object.entries(json.servers)
        : [["server", json]];
  return entries.map(([name, c]) => {
    const strings = (v: unknown) => (Array.isArray(v) ? v.map(String) : []);
    const map = (v: unknown) =>
      v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, String(x)])) : {};
    let transport: McpTransport;
    if (typeof c.command === "string") {
      transport = { type: "stdio", command: c.command, args: strings(c.args), env: map(c.env), ...(typeof c.cwd === "string" ? { cwd: c.cwd } : {}) };
    } else if (typeof c.url === "string" || typeof c.serverUrl === "string") {
      transport = { type: "http", url: String(c.url ?? c.serverUrl), headers: map(c.headers) };
    } else {
      throw new Error(`${name} has neither a command nor a url.`);
    }
    return { id: slug(name) || "server", name, enabled: true, members: false, transport };
  });
}

function Status({ s }: { s: McpServerInfo }) {
  switch (s.status.state) {
    case "ready":
      return <Badge kind="ok">{s.tools.length === 1 ? "1 tool" : `${s.tools.length} tools`}</Badge>;
    case "starting":
      return <Badge>Starting…</Badge>;
    case "error":
      return <Badge kind="warn">Not working</Badge>;
    default:
      return <Badge>Off</Badge>;
  }
}

function PairsEditor({
  label,
  hint,
  value,
  onChange,
  keyHint,
  addLabel,
}: {
  label: string;
  hint: string;
  value: Pair[];
  onChange: (p: Pair[]) => void;
  keyHint: string;
  addLabel: string;
}) {
  const [shown, setShown] = useState(false);
  const set = (i: number, p: Partial<Pair>) => onChange(value.map((x, k) => (k === i ? { ...x, ...p } : x)));
  return (
    <div className="field-block">
      <div className="row spread">
        <strong>{label}</strong>
        {value.length > 0 && (
          <button className="ghost small" onClick={() => setShown(!shown)}>
            {shown ? "Hide values" : "Show values"}
          </button>
        )}
      </div>
      <span className="muted small">{hint}</span>
      {value.map((p, i) => (
        <div key={i} className="pair">
          <input className="mono" placeholder={keyHint} value={p.key} onChange={(e) => set(i, { key: e.target.value })} />
          <input className="mono" type={shown ? "text" : "password"} autoComplete="off" placeholder="value" value={p.value} onChange={(e) => set(i, { value: e.target.value })} />
          <button className="danger-text" aria-label="Remove" onClick={() => onChange(value.filter((_, k) => k !== i))}>
            ×
          </button>
        </div>
      ))}
      <div>
        <button onClick={() => onChange([...value, { key: "", value: "" }])}>{addLabel}</button>
      </div>
    </div>
  );
}

export function ToolsPage() {
  const { remote } = useHost();
  const [starting, setStarting] = useState(false);
  const list = useLoad<McpServerInfo[]>(
    async () => {
      const servers = await remote.mcpServers();
      setStarting(servers.some((s) => s.status.state === "starting"));
      return servers;
    },
    [remote],
    starting ? 1500 : 10000,
  );
  const [form, setForm] = useState<Form | null>(null);
  const [pasting, setPasting] = useState<string | null>(null);
  const action = useAction();

  async function save() {
    if (!form) return;
    const done = await action.run(() => remote.saveMcpServer(serverOf(form)));
    if (done) {
      setForm(null);
      list.reload();
    }
  }

  async function addPasted() {
    if (pasting === null) return;
    const done = await action.run(async () => {
      const servers = parseConfig(pasting);
      if (servers.length === 1) {
        // One server: check it over in the form first.
        setForm(formOf(servers[0], false));
        return true;
      }
      for (const s of servers) await remote.saveMcpServer(s);
      list.reload();
      return true;
    });
    if (done) setPasting(null);
  }

  async function remove(s: McpServerInfo) {
    if (!confirm(`Remove ${s.name}? Models can no longer use its tools.`)) return;
    await action.run(() => remote.deleteMcpServer(s.id));
    list.reload();
  }

  const busyRun = (f: () => Promise<unknown>) => action.run(async () => (await f(), list.reload()));
  const set = (patch: Partial<Form>) => form && setForm({ ...form, ...patch });

  return (
    <div className="page">
      <PageHeader
        title="Tools"
        subtitle="Connect MCP servers so models can read files, fetch pages, search, call APIs and more. Models that support tools use them in the chat on their own."
      />
      <Banner kind="info">
        A server you add as a command runs on this computer with your account's access, so add only servers you trust. Only admins' chats use a server unless you share
        it with members. Small local models are hit-and-miss at using tools; larger ones (7B and up) and cloud models do better.
      </Banner>
      {action.error && <Banner>{action.error}</Banner>}
      {list.error && <Banner>{list.error}</Banner>}

      {(list.value?.length ?? 0) > 0 && (
        <Card title="Servers">
          <div className="provider-list">
            {list.value!.map((s) => (
              <div key={s.id} className="provider mcp-server">
                <label className="switch" title={s.enabled ? "On" : "Off"}>
                  <input type="checkbox" checked={s.enabled} onChange={(e) => busyRun(() => remote.setMcpServerEnabled(s.id, e.target.checked))} />
                  <span />
                </label>
                <div className="provider-main grow">
                  <div>
                    <strong>{s.name}</strong> <Status s={s} /> {s.members ? <Badge>everyone</Badge> : <Badge>admins only</Badge>}
                  </div>
                  <span className="muted small mono wrap-anywhere">
                    {s.transport.type === "stdio" ? [s.transport.command, ...s.transport.args].join(" ") : s.transport.url}
                  </span>
                  {s.status.state === "error" && <div className="mcp-error">{s.status.message}</div>}
                  {s.tools.length > 0 && (
                    <details className="mcp-tools">
                      <summary className="muted small">
                        {s.tools.map((t) => t.name).join(", ")}
                        {s.serverInfo ? ` · ${s.serverInfo}` : ""}
                      </summary>
                      <ul>
                        {s.tools.map((t) => (
                          <li key={t.name}>
                            <code>{t.name}</code> <span className="muted small">{t.description}</span>
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                </div>
                <div className="actions">
                  {s.enabled && (
                    <button disabled={s.status.state === "starting"} onClick={() => busyRun(() => remote.restartMcpServer(s.id))}>
                      Restart
                    </button>
                  )}
                  <button onClick={() => setForm(formOf(s, true))}>Edit</button>
                  <button className="danger-text" onClick={() => remove(s)}>
                    Remove
                  </button>
                </div>
              </div>
            ))}
          </div>
        </Card>
      )}

      {!form && pasting === null && (
        <Card title="Add a server">
          <div className="preset-grid">
            <button className="preset" onClick={() => setForm(blank("stdio"))}>
              <strong>A command</strong>
              <span className="muted small">Runs on this computer, e.g. npx or uvx</span>
            </button>
            <button className="preset" onClick={() => setForm(blank("http"))}>
              <strong>A server address</strong>
              <span className="muted small">Reached over HTTP, e.g. https://…/mcp</span>
            </button>
            <button className="preset" onClick={() => setPasting("")}>
              <strong>Paste a config</strong>
              <span className="muted small">The JSON from a server's instructions</span>
            </button>
          </div>
          <p className="muted small">Or start from one of these:</p>
          <div className="preset-grid">
            {EXAMPLES.map((e) => (
              <button key={e.id} className="preset" onClick={() => setForm(formOf({ id: e.id, name: e.name, enabled: true, members: false, transport: e.transport }, false))}>
                <strong>{e.name}</strong>
                <span className="muted small">{e.about}</span>
              </button>
            ))}
          </div>
        </Card>
      )}

      {pasting !== null && !form && (
        <Card title="Paste a config" actions={<button onClick={() => setPasting(null)}>Cancel</button>}>
          <div className="form">
            <label>
              The JSON a server's instructions give for Claude Desktop, Cursor or VS Code
              <textarea
                className="mono"
                rows={10}
                value={pasting}
                placeholder={'{\n  "mcpServers": {\n    "files": {\n      "command": "npx",\n      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/Documents"]\n    }\n  }\n}'}
                onChange={(e) => setPasting(e.target.value)}
              />
            </label>
            <div className="actions">
              <button className="primary" disabled={action.busy || !pasting.trim()} onClick={addPasted}>
                Add
              </button>
            </div>
          </div>
        </Card>
      )}

      {form && (
        <Card title={form.editing ? `Edit ${form.name}` : "Add a server"} actions={<button onClick={() => setForm(null)}>Cancel</button>}>
          <div className="form">
            <div className="form-row">
              <label>
                Name
                <input
                  value={form.name}
                  placeholder="Files"
                  onChange={(e) => set({ name: e.target.value, ...(form.editing || (form.id && form.id !== slug(form.name)) ? {} : { id: slug(e.target.value) }) })}
                />
              </label>
              <label>
                Short id
                <input value={form.id} disabled={form.editing} placeholder="files" onChange={(e) => set({ id: slug(e.target.value) })} />
              </label>
            </div>
            {form.type === "stdio" ? (
              <>
                <label>
                  Command
                  <input className="mono" value={form.command} placeholder="npx" onChange={(e) => set({ command: e.target.value })} />
                  <span className="muted small">It must be installed on this computer: npx comes with Node.js, uvx with uv.</span>
                </label>
                <label>
                  Arguments, one per line
                  <textarea className="mono" rows={4} value={form.args} placeholder={"-y\n@modelcontextprotocol/server-filesystem\n/Users/me/Documents"} onChange={(e) => set({ args: e.target.value })} />
                </label>
                <PairsEditor
                  label="Environment variables"
                  hint="For API keys and settings the server reads, e.g. GITHUB_PERSONAL_ACCESS_TOKEN."
                  keyHint="NAME"
                  addLabel="Add a variable"
                  value={form.env}
                  onChange={(env) => set({ env })}
                />
                <label>
                  Folder to run it in (optional)
                  <input className="mono" value={form.cwd} placeholder="Where the host runs" onChange={(e) => set({ cwd: e.target.value })} />
                </label>
              </>
            ) : (
              <>
                <label>
                  Server address
                  <input className="mono" value={form.url} placeholder="https://example.org/mcp" onChange={(e) => set({ url: e.target.value })} />
                </label>
                <PairsEditor
                  label="Headers"
                  hint="Sent with every request, e.g. Authorization with the value Bearer and your token."
                  keyHint="Authorization"
                  addLabel="Add a header"
                  value={form.headers}
                  onChange={(headers) => set({ headers })}
                />
              </>
            )}
            <label className="check">
              <input type="checkbox" checked={form.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
              On: start it now and offer its tools in the chat
            </label>
            <label className="check">
              <input type="checkbox" checked={form.members} onChange={(e) => set({ members: e.target.checked })} />
              Members' chats can use it too, not only admins'
            </label>
            <div className="actions">
              <button className="primary" disabled={action.busy || !form.id || (form.type === "stdio" ? !form.command.trim() : !form.url.trim())} onClick={save}>
                {form.editing ? "Save" : "Add"}
              </button>
            </div>
          </div>
        </Card>
      )}

    </div>
  );
}
