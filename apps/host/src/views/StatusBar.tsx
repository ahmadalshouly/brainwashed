import type { EngineState } from "@brainwashed/api";
import { formatBytes } from "../engine";

export function StatusBar({ state }: { state: EngineState }) {
  switch (state.state) {
    case "idle":
      return <div className="status">No model loaded</div>;
    case "installingRuntime":
      return (
        <div className="status busy">
          Installing llama.cpp
          {state.total ? ` ${Math.round((state.done / state.total) * 100)}%` : ` ${formatBytes(state.done)}`}
        </div>
      );
    case "loading":
      return <div className="status busy">Loading {state.model}</div>;
    case "ready":
      return <div className="status ok">Running {state.model}</div>;
    case "error":
      return (
        <div className="status error" title={state.message}>
          Model failed to load
        </div>
      );
  }
}
