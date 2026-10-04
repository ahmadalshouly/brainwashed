import { useState } from "react";
import type { SkillList } from "@brainwashed/api";
import { Badge, Banner, Card, PageHeader, useAction, useHost, useLoad } from "../ui";

const TEMPLATE = `---
name: my-skill
description: One sentence on what this skill does and when to use it.
triggers: [words, that suggest, this skill]
version: 1
---

Instructions for the model when this skill applies:

1. ...
2. ...
`;

export function SkillsPage() {
  const { remote } = useHost();
  const list = useLoad<SkillList>(() => remote.skillList(), [remote], 10000);
  const action = useAction();
  const [editing, setEditing] = useState<{ name: string | null; source: string } | null>(null);

  async function edit(name: string) {
    const source = await action.run(() => remote.skillSource(name));
    if (source !== undefined) setEditing({ name, source });
  }

  async function save() {
    if (!editing) return;
    const saved = await action.run(() => remote.saveSkill(editing.source, editing.name ?? undefined));
    if (saved !== undefined) {
      setEditing(null);
      list.reload();
    }
  }

  if (editing) {
    return (
      <div className="page">
        <PageHeader
          title={editing.name ? `Edit ${editing.name}` : "New skill"}
          subtitle="A skill is a SKILL.md file: a short header, then instructions in Markdown. BrainWashed adds it to the prompt when a message matches."
          actions={
            <>
              <button onClick={() => setEditing(null)}>Cancel</button>
              <button className="primary" disabled={action.busy} onClick={save}>
                Save
              </button>
            </>
          }
        />
        {action.error && <Banner>{action.error}</Banner>}
        <textarea
          className="editor"
          spellCheck={false}
          value={editing.source}
          onChange={(e) => setEditing({ ...editing, source: e.target.value })}
        />
      </div>
    );
  }

  return (
    <div className="page">
      <PageHeader
        title="Skills"
        subtitle={
          <>
            Teach the model how to do specific jobs. Skills live in <span className="mono">{list.value?.dir ?? "…"}</span>{" "}
            and changes there apply right away.
          </>
        }
        actions={
          <button className="primary" onClick={() => setEditing({ name: null, source: TEMPLATE })}>
            New skill
          </button>
        }
      />
      {action.error && <Banner>{action.error}</Banner>}
      {list.value?.errors.map((e) => (
        <Banner key={e.path}>
          <span className="mono">{e.path}</span>: {e.message}
        </Banner>
      ))}
      <Card>
        {list.value?.skills.length === 0 && <p className="muted">No skills yet.</p>}
        <div className="list">
          {list.value?.skills.map((s) => (
            <div key={s.name} className="list-row">
              <label className="switch" title={s.enabled ? "On" : "Off"}>
                <input
                  type="checkbox"
                  checked={s.enabled}
                  onChange={(e) => action.run(async () => (await remote.setSkillEnabled(s.name, e.target.checked), list.reload()))}
                />
                <span />
              </label>
              <div className="grow">
                <strong>{s.name}</strong> {!s.enabled && <Badge>off</Badge>}
                <div className="muted small">{s.description}</div>
                {s.triggers.length > 0 && (
                  <div className="tags">
                    {s.triggers.map((t) => (
                      <span key={t} className="tag">
                        {t}
                      </span>
                    ))}
                  </div>
                )}
              </div>
              <button onClick={() => edit(s.name)}>Edit</button>
              <button
                className="danger-text"
                onClick={() => {
                  if (confirm(`Delete the skill ${s.name}?`))
                    action.run(async () => (await remote.deleteSkill(s.name), list.reload()));
                }}
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}
