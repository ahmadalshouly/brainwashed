import { useEffect, useRef, useState } from "react";
import { HostReplyError, type ChatMessage, type InstalledModel } from "@brainwashed/api";
import { Markdown } from "../markdown";
import { loadChats, newId, saveChats, titleFor, type Conversation } from "../storage";
import { errorText, stateText, useHost, useLoad } from "../ui";

interface Turn extends ChatMessage {
  skills?: string[];
  reasoning?: string;
  error?: string;
}

const plain = (turns: Turn[]): ChatMessage[] =>
  turns.filter((t) => !t.error && t.content).map(({ role, content }) => ({ role, content }));

const SUGGESTIONS = [
  "Plan my meals for this week",
  "Explain how a VPN works, simply",
  "Write a polite email asking for a deadline extension",
  "Summarize the pros and cons of remote work",
];

export function ChatPage() {
  const { remote, role, state, refreshState, fail } = useHost();
  const models = useLoad<InstalledModel[]>(() => remote.models(), [remote], 15000);
  const [chats, setChats] = useState<Conversation[]>(loadChats);
  const [activeId, setActiveId] = useState<string | null>(() => loadChats()[0]?.id ?? null);
  const [turns, setTurns] = useState<Turn[]>(() => loadChats()[0]?.messages ?? []);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [showList, setShowList] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);

  const ready = state?.state === "ready";
  const current = state && "model" in state ? state.model : null;
  const modelName = (id: string) => models.value?.find((m) => m.id === id)?.name ?? id;

  // Keep the active conversation saved as it changes.
  useEffect(() => {
    const messages = plain(turns);
    if (!messages.length) return;
    let id = activeId;
    if (!id) {
      id = newId();
      setActiveId(id);
    }
    const chatId = id;
    setChats((all) => {
      const id = chatId;
      const others = all.filter((c) => c.id !== id);
      const prev = all.find((c) => c.id === id);
      const next = [{ id, title: prev?.title && prev.title !== "New chat" ? prev.title : titleFor(messages), messages, updatedAt: Date.now() }, ...others];
      saveChats(next);
      return next;
    });
    // Only when the conversation itself changes.
  }, [turns]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: busy ? "auto" : "smooth", block: "end" });
  }, [turns, busy]);

  function open(c: Conversation | null) {
    if (busy) abort.current?.abort();
    setActiveId(c?.id ?? null);
    setTurns(c?.messages ?? []);
    setShowList(false);
    setTimeout(() => box.current?.focus(), 0);
  }

  function remove(id: string) {
    const next = chats.filter((c) => c.id !== id);
    setChats(next);
    saveChats(next);
    if (id === activeId) open(next[0] ?? null);
  }

  async function ask(history: ChatMessage[]) {
    setTurns([...history, { role: "assistant", content: "" }]);
    setBusy(true);
    const controller = new AbortController();
    abort.current = controller;
    const update = (f: (t: Turn) => Turn) => setTurns((all) => [...all.slice(0, -1), f(all[all.length - 1])]);
    try {
      await remote.chat(
        history,
        (e) =>
          update((t) => {
            switch (e.kind) {
              case "skills":
                return { ...t, skills: e.names };
              case "content":
                return { ...t, content: t.content + e.text };
              case "reasoning":
                return { ...t, reasoning: (t.reasoning ?? "") + e.text };
              default:
                return t;
            }
          }),
        controller.signal,
      );
    } catch (e) {
      if (!controller.signal.aborted) {
        if (e instanceof HostReplyError && e.status === 401) fail(e);
        else update((t) => ({ ...t, error: errorText(e) }));
      }
    } finally {
      abort.current = null;
      setBusy(false);
    }
  }

  function send(text = input) {
    text = text.trim();
    if (!text || busy || !ready) return;
    setInput("");
    ask([...plain(turns), { role: "user", content: text }]);
  }

  function regenerate() {
    const history = plain(turns);
    while (history.length && history[history.length - 1].role === "assistant") history.pop();
    if (history.length) ask(history);
  }

  async function pickModel(id: string) {
    try {
      await remote.loadModel(id);
      refreshState();
    } catch (e) {
      fail(e);
    }
  }

  const last = turns[turns.length - 1];

  return (
    <div className={`chat ${showList ? "list-open" : ""}`}>
      <aside className="chat-list">
        <button className="primary block" onClick={() => open(null)}>
          + New chat
        </button>
        <div className="chat-items">
          {chats.length === 0 && <p className="muted small pad">Your conversations appear here. They're kept only in this browser.</p>}
          {chats.map((c) => (
            <div key={c.id} className={`chat-item ${c.id === activeId ? "active" : ""}`}>
              <button className="ghost" onClick={() => open(c)} title={c.title}>
                {c.title}
              </button>
              <button className="ghost icon" aria-label="Delete chat" onClick={() => remove(c.id)}>
                ×
              </button>
            </div>
          ))}
        </div>
      </aside>

      <section className="chat-main">
        <header className="chat-head">
          <button className="ghost list-toggle" onClick={() => setShowList(!showList)}>
            Chats
          </button>
          <div className="chat-model">
            {role === "admin" && (models.value?.length ?? 0) > 0 ? (
              <select
                aria-label="Model"
                value={current ?? ""}
                disabled={busy || state?.state === "loading"}
                onChange={(e) => pickModel(e.target.value)}
              >
                {!current && <option value="">Choose a model</option>}
                {models.value!.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            ) : (
              <span className="muted">{stateText(state, modelName)}</span>
            )}
            {state?.state === "loading" && <span className="muted small">Loading…</span>}
          </div>
        </header>

        <div className="messages">
          {turns.length === 0 && (
            <div className="empty">
              <h2>{ready ? "What can I help with?" : stateText(state, modelName)}</h2>
              <p className="muted">
                {ready
                  ? "Runs privately on your own computer. Nothing leaves it."
                  : role === "admin"
                    ? "Pick or download a model in Models to start chatting."
                    : "An admin needs to start a model before you can chat."}
              </p>
              {ready && (
                <div className="suggestions">
                  {SUGGESTIONS.map((s) => (
                    <button key={s} onClick={() => send(s)}>
                      {s}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
          {turns.map((t, i) => (
            <div key={i} className={`turn ${t.role}`}>
              {t.role === "user" ? (
                <div className="bubble user">{t.content}</div>
              ) : (
                <div className="answer">
                  {t.skills && t.skills.length > 0 && <div className="skills-used">Using {t.skills.join(", ")}</div>}
                  {t.reasoning && (
                    <details className="reasoning">
                      <summary>{busy && i === turns.length - 1 && !t.content ? "Thinking…" : "Thought process"}</summary>
                      <div className="reasoning-body">{t.reasoning}</div>
                    </details>
                  )}
                  {t.content ? (
                    <Markdown text={t.content} />
                  ) : busy && i === turns.length - 1 && !t.reasoning ? (
                    <span className="typing">
                      <i />
                      <i />
                      <i />
                    </span>
                  ) : null}
                  {t.error && <div className="banner error">{t.error}</div>}
                  {!busy && t.content && (
                    <div className="turn-actions">
                      <button className="ghost small" onClick={() => navigator.clipboard?.writeText(t.content)}>
                        Copy
                      </button>
                      {i === turns.length - 1 && ready && (
                        <button className="ghost small" onClick={regenerate}>
                          Regenerate
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}
          {last?.error && !busy && ready && (
            <button className="retry" onClick={regenerate}>
              Try again
            </button>
          )}
          <div ref={bottom} />
        </div>

        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <textarea
            ref={box}
            value={input}
            placeholder={ready ? "Message BrainWashed" : "Waiting for a model…"}
            disabled={!ready}
            rows={1}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !matchMedia("(pointer: coarse)").matches) {
                e.preventDefault();
                send();
              }
            }}
          />
          {busy ? (
            <button type="button" onClick={() => abort.current?.abort()}>
              Stop
            </button>
          ) : (
            <button className="primary" type="submit" disabled={!ready || !input.trim()}>
              Send
            </button>
          )}
        </form>
      </section>
    </div>
  );
}
