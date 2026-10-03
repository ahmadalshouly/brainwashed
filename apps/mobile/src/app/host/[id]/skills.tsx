import { useCallback, useEffect, useMemo, useState } from "react";
import { FlatList, StyleSheet, Switch, Text, View } from "react-native";
import { useLocalSearchParams } from "expo-router";
import type { SkillInfo } from "@brainwashed/api";
import { useHosts } from "../../../lib/hosts";
import { useTheme } from "../../../lib/theme";

export default function Skills() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { remote: makeRemote } = useHosts();
  const remote = useMemo(() => makeRemote(id), [id, makeRemote]);
  const t = useTheme();
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!remote) return;
    try {
      setSkills(await remote.skills());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [remote]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const toggle = async (name: string, enabled: boolean) => {
    setSkills((all) => all.map((s) => (s.name === name ? { ...s, enabled } : s)));
    try {
      await remote?.setSkillEnabled(name, enabled);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refresh();
    }
  };

  return (
    <FlatList
      data={skills}
      keyExtractor={(s) => s.name}
      contentContainerStyle={styles.list}
      ListHeaderComponent={
        <View style={{ gap: 8 }}>
          {error && <Text style={{ color: t.danger }}>{error}</Text>}
          <Text style={{ color: t.muted }}>
            The model uses a skill automatically when your message matches it. Write new skills in the desktop app.
          </Text>
        </View>
      }
      renderItem={({ item }) => (
        <View style={[styles.row, { backgroundColor: t.panel, borderColor: t.border }]}>
          <View style={{ flex: 1, gap: 4 }}>
            <Text style={[styles.title, { color: t.text }]}>{item.name}</Text>
            <Text style={{ color: t.muted }}>{item.description}</Text>
          </View>
          <Switch value={item.enabled} onValueChange={(v) => toggle(item.name, v)} />
        </View>
      )}
    />
  );
}

const styles = StyleSheet.create({
  list: { padding: 16, gap: 10 },
  row: { flexDirection: "row", alignItems: "center", gap: 12, padding: 14, borderRadius: 12, borderWidth: 1 },
  title: { fontSize: 16, fontWeight: "600" },
});
