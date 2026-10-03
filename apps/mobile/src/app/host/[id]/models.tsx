import { useCallback, useEffect, useMemo, useState } from "react";
import { FlatList, Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";
import { useLocalSearchParams } from "expo-router";
import type { EngineState, InstalledModel } from "@brainwashed/api";
import { useHosts } from "../../../lib/hosts";
import { useTheme } from "../../../lib/theme";

function formatBytes(n: number) {
  return n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.round(n / 1e6)} MB`;
}

export default function Models() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { remote: makeRemote } = useHosts();
  const remote = useMemo(() => makeRemote(id), [id, makeRemote]);
  const t = useTheme();
  const [models, setModels] = useState<InstalledModel[]>([]);
  const [state, setState] = useState<EngineState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const refresh = useCallback(async () => {
    if (!remote) return;
    try {
      const [m, s] = await Promise.all([remote.models(), remote.state()]);
      setModels(m);
      setState(s);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [remote]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 3000);
    return () => clearInterval(timer);
  }, [refresh]);

  const active = state && (state.state === "ready" || state.state === "loading") ? state.model : null;

  return (
    <FlatList
      data={models}
      keyExtractor={(m) => m.id}
      contentContainerStyle={styles.list}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={async () => {
            setRefreshing(true);
            await refresh();
            setRefreshing(false);
          }}
        />
      }
      ListHeaderComponent={
        <View style={{ gap: 8 }}>
          {error && <Text style={{ color: t.danger }}>{error}</Text>}
          <Text style={{ color: t.muted }}>
            Models installed on the computer. Download new ones from the desktop app.
          </Text>
        </View>
      }
      renderItem={({ item }) => {
        const isActive = item.id === active;
        return (
          <Pressable
            style={[styles.row, { backgroundColor: t.panel, borderColor: isActive ? t.accent : t.border }]}
            disabled={isActive}
            onPress={() => remote?.loadModel(item.id).then(refresh, (e) => setError(String(e)))}
          >
            <Text style={[styles.title, { color: t.text }]}>{item.name}</Text>
            <Text style={{ color: isActive ? t.accent : t.muted }}>
              {isActive ? (state?.state === "loading" ? "Loading…" : "Running") : `Tap to run · ${formatBytes(item.size)}`}
            </Text>
          </Pressable>
        );
      }}
    />
  );
}

const styles = StyleSheet.create({
  list: { padding: 16, gap: 10 },
  row: { padding: 14, borderRadius: 12, borderWidth: 1, gap: 4 },
  title: { fontSize: 16, fontWeight: "600" },
});
