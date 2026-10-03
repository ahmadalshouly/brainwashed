import { useCallback, useEffect, useState } from "react";
import type { SkillList } from "@brainwashed/api";
import { engine, errorText, onEngineEvent } from "../engine";

const TEMPLATE = `---
name: my-skill
description: One line saying what this skill does and when to use it.
triggers: [phrase that should switch it on]
version: 1
---

When the user asks for this:

1. First step.
2. Second step.

Keep instructions short and concrete. Small models follow short lists best.
`;

/** `null` while browsing; otherwise the skill being edited. */
type Editing = { previousName?: string; source: string } | null;

export function SkillsView() {
  const [list, setList] = useState<SkillList | null>(null);
  const [editing, setEditing] = useState<Editing>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    engine.skills().then(setList);
  }, []);

  useEffect(() => {
    refresh();
    return onEngineEvent((e) => {
      if (e.type === "skillsChanged") refresh();
    });
  }, [refresh]);

  const run = async (p: Promise<unknown>, after?: () => void) => {
    setError(null);
    try {
      await p;
      after?.();
      refresh();
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (editing) {
    return (
      <div className="skills">
        <h2>{editing.previousName ? `Edit ${editing.previousName}` : "New skill"}</h2>
        {error && <div className="banner error">{error}</div>}
        <textarea
          className="editor"
          spellCheck={false}
          value={editing.source}
          onChange={(e) => setEditing({ ...editing, source: e.target.value })}
        />
        <div className="row">
          <button
            className="primary"
            onClick={() => run(engine.saveSkill(editing.source, editing.previousName), () => setEditing(null))}
          >
            Save
          </button>
          <button
            onClick={() => {
              setError(null);
              setEditing(null);
            }}
          >
            Cancel
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="skills">
      <div className="row header">
        <h2>Skills</h2>
        <button className="primary" onClick={() => setEditing({ source: TEMPLATE })}>
          New skill
        </button>
      </div>
      <p className="muted">
        A skill is a <code>SKILL.md</code> file that teaches the model how to handle a kind of request. The
        model gets the right skill automatically when your message matches it. You can also edit files in{" "}
        <code>{list?.dir}</code> with any text editor; changes apply right away.
      </p>
      {error && <div className="banner error">{error}</div>}
      {list?.errors.map((e) => (
        <div key={e.path} className="banner error">
          {e.path}: {e.message}
        </div>
      ))}
      <ul className="list">
        {list?.skills.map((s) => (
          <li key={s.name}>
            <div>
              <strong>{s.name}</strong>
              {!s.enabled && <span className="tag">Off</span>}
              <div className="muted">{s.description}</div>
              {s.triggers.length > 0 && (
                <div className="muted small">Triggers: {s.triggers.join(", ")}</div>
              )}
            </div>
            <div className="actions">
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={s.enabled}
                  onChange={(e) => run(engine.setSkillEnabled(s.name, e.target.checked))}
                />
                On
              </label>
              <button
                onClick={() =>
                  run(
                    engine.skillSource(s.name).then((source) => setEditing({ previousName: s.name, source })),
                  )
                }
              >
                Edit
              </button>
              <button onClick={() => run(engine.deleteSkill(s.name))}>Delete</button>
            </div>
          </li>
        ))}
      </ul>
      {list && list.skills.length === 0 && <p className="hint">No skills yet.</p>}
    </div>
  );
}
