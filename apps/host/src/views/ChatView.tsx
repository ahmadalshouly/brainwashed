import { useEffect, useRef, useState } from "react";
import type { ChatMessage, EngineState } from "@brainwashed/api";
import { engine, errorText } from "../engine";

interface Turn extends ChatMessage {
  reasoning?: string;
  skills?: string[];
  error?: string;
}

export function ChatView({ state, onPickModel }: { state: EngineState; onPickModel: () => void }) {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

  useEffect(() => bottom.current?.scrollIntoView({ behavior: "smooth" }), [turns]);

  const ready = state.state === "ready";

  async function send() {
    const text = input.trim();
    if (!text || busy || !ready) return;
    const history: ChatMessage[] = [
      ...turns.filter((t) => !t.error).map(({ role, content }) => ({ role, content })),
      { role: "user", content: text },
    ];
    setTurns([...history, { role: "assistant", content: "" }]);
    setInput("");
    setBusy(true);

    const update = (f: (t: Turn) => Turn) =>
      setTurns((all) => [...all.slice(0, -1), f(all[all.length - 1])]);
    try {
      await engine.chat(history, (e) =>
        update((t) => {
          switch (e.kind) {
            case "skills":
              return { ...t, skills: e.names };
            case "content":
              return { ...t, content: t.content + e.text };
            case "reasoning":
              return { ...t, reasoning: (t.reasoning ?? "") + e.text };
          }
        }),
      );
    } catch (e) {
      update((t) => ({ ...t, error: errorText(e) }));
    } finally {
      setBusy(false);
    }
  }

  if (!ready && turns.length === 0) {
    return (
      <div className="empty">
        <h2>No model running</h2>
        <p>Download or load a model to start chatting.</p>
        <button className="primary" onClick={onPickModel}>
          Choose a model
        </button>
      </div>
    );
  }

  return (
    <div className="chat">
      <div className="messages">
        {turns.length === 0 && <p className="hint">Ask anything. Everything stays on this computer.</p>}
        {turns.map((t, i) => (
          <div key={i} className={`bubble ${t.role}`}>
            {t.skills && t.skills.length > 0 && (
              <div className="skills-used">Using {t.skills.join(", ")}</div>
            )}
            {t.reasoning && (
              <details className="reasoning">
                <summary>Thinking</summary>
                {t.reasoning}
              </details>
            )}
            {t.content || (busy && i === turns.length - 1 ? <span className="typing">…</span> : null)}
            {t.error && <div className="error">{t.error}</div>}
          </div>
        ))}
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
          value={input}
          placeholder={ready ? "Message BrainWashed" : "Load a model to chat"}
          disabled={!ready}
          rows={2}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        <button className="primary" type="submit" disabled={!ready || busy || !input.trim()}>
          Send
        </button>
        {turns.length > 0 && (
          <button type="button" disabled={busy} onClick={() => setTurns([])}>
            New chat
          </button>
        )}
      </form>
    </div>
  );
}
