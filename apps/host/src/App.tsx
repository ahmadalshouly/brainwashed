import { useState } from "react";
import { ChatView } from "./views/ChatView";
import { ModelsView } from "./views/ModelsView";
import { StatusBar } from "./views/StatusBar";
import { useEngineState } from "./engine";

type Tab = "chat" | "models";

export function App() {
  const state = useEngineState();
  const [tab, setTab] = useState<Tab>(state.state === "ready" ? "chat" : "models");

  return (
    <div className="app">
      <nav className="sidebar">
        <h1>BrainWashed</h1>
        <button className={tab === "chat" ? "active" : ""} onClick={() => setTab("chat")}>
          Chat
        </button>
        <button className={tab === "models" ? "active" : ""} onClick={() => setTab("models")}>
          Models
        </button>
        <div className="spacer" />
        <StatusBar state={state} />
      </nav>
      <main className="content">
        {tab === "chat" ? (
          <ChatView state={state} onPickModel={() => setTab("models")} />
        ) : (
          <ModelsView state={state} />
        )}
      </main>
    </div>
  );
}
