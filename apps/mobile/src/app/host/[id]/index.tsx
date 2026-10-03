import { useEffect, useMemo, useRef, useState } from "react";
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { Stack, router, useLocalSearchParams } from "expo-router";
import type { ChatMessage, EngineState } from "@brainwashed/api";
import { useHosts } from "../../../lib/hosts";
import { useTheme } from "../../../lib/theme";

interface Turn extends ChatMessage {
  skills?: string[];
  reasoning?: string;
  error?: string;
}

export default function Chat() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { hosts, remote: makeRemote } = useHosts();
  const remote = useMemo(() => makeRemote(id), [id, makeRemote]);
  const host = hosts?.find((h) => h.hostId === id);
  const t = useTheme();

  const [state, setState] = useState<EngineState | null>(null);
  const [offline, setOffline] = useState<string | null>(null);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const list = useRef<FlatList<Turn>>(null);

  // Keep the model status fresh, e.g. while a model loads.
  useEffect(() => {
    if (!remote) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const s = await remote.state();
        if (!cancelled) {
          setState(s);
          setOffline(null);
        }
      } catch (e) {
        if (!cancelled) setOffline(e instanceof Error ? e.message : String(e));
      }
    };
    poll();
    const timer = setInterval(poll, 4000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [remote]);

  if (!remote || !host) {
    return <Text style={{ color: t.muted, padding: 24 }}>This computer is no longer paired.</Text>;
  }

  const ready = state?.state === "ready";

  const send = async () => {
    const text = input.trim();
    if (!text || busy || !ready) return;
    const history: ChatMessage[] = [
      ...turns.filter((x) => !x.error).map(({ role, content }) => ({ role, content })),
      { role: "user", content: text },
    ];
    setTurns([...history, { role: "assistant", content: "" }]);
    setInput("");
    setBusy(true);
    abort.current = new AbortController();
    const update = (f: (x: Turn) => Turn) => setTurns((all) => [...all.slice(0, -1), f(all[all.length - 1])]);
    try {
      await remote.chat(
        history,
        (e) =>
          update((x) => {
            if (e.kind === "skills") return { ...x, skills: e.names };
            if (e.kind === "content") return { ...x, content: x.content + e.text };
            return { ...x, reasoning: (x.reasoning ?? "") + e.text };
          }),
        abort.current.signal,
      );
    } catch (e) {
      if (!abort.current?.signal.aborted) {
        update((x) => ({ ...x, error: e instanceof Error ? e.message : String(e) }));
      }
    } finally {
      setBusy(false);
      abort.current = null;
    }
  };

  return (
    <KeyboardAvoidingView
      style={{ flex: 1 }}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
      keyboardVerticalOffset={90}
    >
      <Stack.Screen
        options={{
          title: host.hostName,
          headerRight: () => (
            <View style={{ flexDirection: "row", gap: 16 }}>
              <Pressable onPress={() => router.push({ pathname: "/host/[id]/models", params: { id } })}>
                <Text style={{ color: t.accent }}>Models</Text>
              </Pressable>
              <Pressable onPress={() => router.push({ pathname: "/host/[id]/skills", params: { id } })}>
                <Text style={{ color: t.accent }}>Skills</Text>
              </Pressable>
            </View>
          ),
        }}
      />
      <StatusLine state={state} offline={offline} />
      <FlatList
        ref={list}
        data={turns}
        keyExtractor={(_, i) => String(i)}
        contentContainerStyle={styles.messages}
        onContentSizeChange={() => list.current?.scrollToEnd({ animated: true })}
        ListEmptyComponent={
          <Text style={{ color: t.muted, textAlign: "center", marginTop: 32 }}>
            {ready ? "Ask anything. It runs on your own computer." : "Waiting for a model on the computer."}
          </Text>
        }
        renderItem={({ item, index }) => (
          <View
            style={[
              styles.bubble,
              item.role === "user"
                ? { alignSelf: "flex-end", backgroundColor: t.user }
                : { alignSelf: "flex-start", backgroundColor: t.panel, borderColor: t.border, borderWidth: 1 },
            ]}
          >
            {item.skills && item.skills.length > 0 && (
              <Text style={{ color: t.accent, fontSize: 12, marginBottom: 4 }}>Using {item.skills.join(", ")}</Text>
            )}
            <Text style={{ color: t.text, fontSize: 16, lineHeight: 22 }}>
              {item.content || (busy && index === turns.length - 1 ? "…" : "")}
            </Text>
            {item.error && <Text style={{ color: t.danger, marginTop: 4 }}>{item.error}</Text>}
          </View>
        )}
      />
      <View style={[styles.composer, { borderColor: t.border, backgroundColor: t.panel }]}>
        <TextInput
          value={input}
          onChangeText={setInput}
          placeholder={ready ? "Message" : "No model running"}
          placeholderTextColor={t.muted}
          editable={ready}
          multiline
          style={[styles.input, { color: t.text }]}
          onSubmitEditing={send}
        />
        {busy ? (
          <Pressable style={[styles.send, { backgroundColor: t.border }]} onPress={() => abort.current?.abort()}>
            <Text style={{ color: t.text }}>Stop</Text>
          </Pressable>
        ) : (
          <Pressable
            style={[styles.send, { backgroundColor: t.accent, opacity: ready && input.trim() ? 1 : 0.5 }]}
            disabled={!ready || !input.trim()}
            onPress={send}
          >
            <Text style={{ color: t.accentText, fontWeight: "600" }}>Send</Text>
          </Pressable>
        )}
      </View>
    </KeyboardAvoidingView>
  );
}

function StatusLine({ state, offline }: { state: EngineState | null; offline: string | null }) {
  const t = useTheme();
  let text: string;
  let color = t.muted;
  if (offline) {
    text = offline;
    color = t.danger;
  } else if (!state) {
    text = "Connecting…";
  } else {
    switch (state.state) {
      case "ready":
        text = `Running ${state.model}`;
        color = t.ok;
        break;
      case "loading":
        text = `Loading ${state.model}…`;
        break;
      case "installingRuntime":
        text = "Setting up the AI runtime on the computer…";
        break;
      case "error":
        text = state.message;
        color = t.danger;
        break;
      default:
        text = "No model running. Pick one in Models.";
    }
  }
  return <Text style={[styles.status, { color }]}>{text}</Text>;
}

const styles = StyleSheet.create({
  status: { paddingHorizontal: 16, paddingVertical: 6, fontSize: 13 },
  messages: { padding: 16, gap: 10, flexGrow: 1 },
  bubble: { maxWidth: "85%", padding: 12, borderRadius: 14 },
  composer: { flexDirection: "row", alignItems: "flex-end", gap: 8, padding: 10, borderTopWidth: 1 },
  input: { flex: 1, fontSize: 16, maxHeight: 120, paddingVertical: 8 },
  send: { paddingHorizontal: 16, paddingVertical: 10, borderRadius: 10 },
});
