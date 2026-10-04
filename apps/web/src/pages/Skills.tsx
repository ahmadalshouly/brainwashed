import { useMemo, useState } from "react";
import type { CommunitySkill, SkillInfo, SkillList, SkillPreview } from "@brainwashed/api";
import { Badge, Banner, Card, PageHeader, useAction, useHost, useLoad } from "../ui";

/** Where people share skills. Submissions are pull requests there. */
const REGISTRY = "https://github.com/ahmadalshouly/brainwashed-skills";

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

/** Opens GitHub's "new file" page in the registry with this skill filled in; GitHub turns it into a pull request. */
function shareUrl(name: string, source: string) {
  const params = new URLSearchParams({ filename: `skills/${name}/SKILL.md`, value: source });
  return `${REGISTRY}/new/main?${params}`;
}

/** An update is out for a skill installed from the community index. */
function hasUpdate(skill: SkillInfo, index: CommunitySkill[] | null) {
  if (!skill.origin?.community || !index) return false;
  const latest = index.find((c) => c.name === skill.name);
  return !!latest && latest.sha256 !== skill.origin.sha256;
}

export function SkillsPage() {
  const { remote } = useHost();
  const list = useLoad<SkillList>(() => remote.skillList(), [remote], 10000);
  // Loaded once; used for update badges and the Community tab.
  const index = useLoad<CommunitySkill[]>(() => remote.communitySkills(), [remote]);
  const action = useAction();
  const [tab, setTab] = useState<"mine" | "community">("mine");
  const [editing, setEditing] = useState<{ name: string | null; source: string } | null>(null);
  const [preview, setPreview] = useState<{ spec: string; skill: SkillPreview } | null>(null);

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

  async function look(spec: string) {
    const skill = await action.run(() => remote.previewSkill(spec));
    if (skill) setPreview({ spec, skill });
  }

  async function install() {
    if (!preview) return;
    const done = await action.run(() => remote.installSkill(preview.spec, preview.skill.sha256, preview.skill.installed));
    if (done) {
      setPreview(null);
      setTab("mine");
      list.reload();
    }
  }

  async function share(name: string) {
    const source = await action.run(() => remote.skillSource(name));
    if (source !== undefined) window.open(shareUrl(name, source), "_blank", "noopener");
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

  if (preview) {
    const s = preview.skill;
    return (
      <div className="page">
        <PageHeader
          title={`Install ${s.name}?`}
          subtitle={
            <>
              Read it first: everything below goes into the model's instructions when a message matches. From{" "}
              <a href={s.url} target="_blank" rel="noopener noreferrer" className="mono">
                {s.url}
              </a>
            </>
          }
          actions={
            <>
              <button onClick={() => setPreview(null)}>Cancel</button>
              <button className="primary" disabled={action.busy} onClick={install}>
                {s.installed ? "Replace mine" : "Install"}
              </button>
            </>
          }
        />
        {action.error && <Banner>{action.error}</Banner>}
        {s.installed && <Banner kind="info">You already have a skill named {s.name}. Installing replaces it.</Banner>}
        {s.warnings.length > 0 ? (
          <Banner>
            Look closely before installing:
            <ul className="warnings">
              {s.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </Banner>
        ) : (
          <Banner kind="ok">Nothing in it looks like an attempt to override your instructions.</Banner>
        )}
        <textarea className="editor" readOnly spellCheck={false} value={s.source} />
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
      <div className="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "mine"} className={tab === "mine" ? "on" : ""} onClick={() => setTab("mine")}>
          Your skills
        </button>
        <button
          role="tab"
          aria-selected={tab === "community"}
          className={tab === "community" ? "on" : ""}
          onClick={() => setTab("community")}
        >
          Community
        </button>
      </div>
      {action.error && <Banner>{action.error}</Banner>}
      {tab === "mine" ? (
        <MySkills list={list} index={index.value} onEdit={edit} onShare={share} onUpdate={look} action={action} />
      ) : (
        <Community index={index} installed={list.value?.skills ?? []} onLook={look} busy={action.busy} />
      )}
    </div>
  );
}

function MySkills({
  list,
  index,
  onEdit,
  onShare,
  onUpdate,
  action,
}: {
  list: ReturnType<typeof useLoad<SkillList>>;
  index: CommunitySkill[] | null;
  onEdit: (name: string) => void;
  onShare: (name: string) => void;
  onUpdate: (name: string) => void;
  action: ReturnType<typeof useAction>;
}) {
  const { remote } = useHost();
  return (
    <>
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
                <strong>{s.name}</strong> {s.builtin && <Badge>built-in</Badge>} {!s.enabled && <Badge>off</Badge>}{" "}
                {s.origin && <Badge>{s.origin.community ? "community" : "from a link"}</Badge>}{" "}
                {s.modified && <Badge kind="warn">edited</Badge>}
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
              {hasUpdate(s, index) && (
                <button
                  className="primary"
                  title={s.modified ? "Updating replaces your changes" : undefined}
                  onClick={() => onUpdate(s.name)}
                >
                  Update
                </button>
              )}
              {!s.origin && !s.builtin && (
                <button title="Share it with everyone who uses BrainWashed" onClick={() => onShare(s.name)}>
                  Share
                </button>
              )}
              <button onClick={() => onEdit(s.name)}>Edit</button>
              {!s.builtin && (
                <button
                  className="danger-text"
                  onClick={() => {
                    if (confirm(`Delete the skill ${s.name}?`))
                      action.run(async () => (await remote.deleteSkill(s.name), list.reload()));
                  }}
                >
                  Delete
                </button>
              )}
            </div>
          ))}
        </div>
      </Card>
      <p className="muted small">
        Share sends your skill to the <a href={REGISTRY} target="_blank" rel="noopener noreferrer">community registry</a> as a
        GitHub pull request. Once it's reviewed, everyone can install it.
      </p>
    </>
  );
}

function Community({
  index,
  installed,
  onLook,
  busy,
}: {
  index: ReturnType<typeof useLoad<CommunitySkill[]>>;
  installed: SkillInfo[];
  onLook: (spec: string) => void;
  busy: boolean;
}) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [link, setLink] = useState("");

  const categories = useMemo(
    () => [...new Set((index.value ?? []).map((s) => s.category).filter((c): c is string => !!c))].sort(),
    [index.value],
  );
  const shown = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return (index.value ?? []).filter((s) => {
      if (category && s.category !== category) return false;
      const text = `${s.name} ${s.description} ${s.triggers.join(" ")} ${s.author ?? ""}`.toLowerCase();
      return words.every((w) => text.includes(w));
    });
  }, [index.value, query, category]);

  return (
    <>
      <Card>
        <div className="row">
          <input
            className="grow"
            type="search"
            placeholder="Search shared skills"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        {categories.length > 0 && (
          <div className="tags chips">
            <button className={category === null ? "chip on" : "chip"} onClick={() => setCategory(null)}>
              All
            </button>
            {categories.map((c) => (
              <button key={c} className={category === c ? "chip on" : "chip"} onClick={() => setCategory(c)}>
                {c}
              </button>
            ))}
          </div>
        )}
      </Card>
      {index.error && (
        <Banner>
          Couldn't load community skills: {index.error} <button onClick={index.reload}>Try again</button>
        </Banner>
      )}
      <Card>
        {!index.value && !index.error && <p className="muted">Loading…</p>}
        {index.value && shown.length === 0 && <p className="muted">No shared skills match.</p>}
        <div className="list">
          {shown.map((s) => {
            const mine = installed.find((i) => i.name === s.name);
            const current = mine?.origin?.sha256 === s.sha256;
            return (
              <div key={s.name} className="list-row">
                <div className="grow">
                  <strong>{s.name}</strong>
                  {s.author && <span className="muted small"> by {s.author}</span>}{" "}
                  {mine && <Badge kind={current ? "ok" : ""}>{current ? "installed" : "you have one"}</Badge>}
                  <div className="muted small">{s.description}</div>
                  {(s.category || s.triggers.length > 0) && (
                    <div className="tags">
                      {s.category && <span className="tag">{s.category}</span>}
                      {s.triggers.map((t) => (
                        <span key={t} className="tag">
                          {t}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                {s.page && (
                  <a className="small" href={s.page} target="_blank" rel="noopener noreferrer">
                    Details
                  </a>
                )}
                <button disabled={busy || current} onClick={() => onLook(s.name)}>
                  {current ? "Installed" : "View"}
                </button>
              </div>
            );
          })}
        </div>
      </Card>
      <Card title="Install from a link">
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            if (link.trim()) onLook(link.trim());
          }}
        >
          <input
            className="grow"
            placeholder="https://github.com/someone/skills/blob/main/my-skill/SKILL.md"
            value={link}
            onChange={(e) => setLink(e.target.value)}
          />
          <button disabled={busy || !link.trim()}>View</button>
        </form>
        <p className="muted small">
          Any SKILL.md on the web works. You'll see the whole skill before anything is installed.
        </p>
      </Card>
    </>
  );
}
