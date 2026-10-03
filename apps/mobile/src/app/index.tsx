import { router } from "expo-router";
import { ActivityIndicator, Alert, Pressable, StyleSheet, Text, View } from "react-native";
import { useHosts } from "../lib/hosts";
import { useTheme } from "../lib/theme";

export default function Hosts() {
  const { hosts, removeHost } = useHosts();
  const t = useTheme();

  if (hosts === null) {
    return <ActivityIndicator style={{ marginTop: 48 }} />;
  }

  if (hosts.length === 0) {
    return (
      <View style={styles.empty}>
        <Text style={[styles.title, { color: t.text }]}>Your AI, at home</Text>
        <Text style={[styles.body, { color: t.muted }]}>
          Open BrainWashed on your computer, go to Phones, turn on phone access and tap Pair a phone. Then scan the
          code with this app.
        </Text>
        <Pressable style={[styles.primary, { backgroundColor: t.accent }]} onPress={() => router.push("/pair")}>
          <Text style={[styles.primaryText, { color: t.accentText }]}>Pair a computer</Text>
        </Pressable>
      </View>
    );
  }

  return (
    <View style={styles.list}>
      {hosts.map((h) => (
        <Pressable
          key={h.hostId}
          style={[styles.row, { backgroundColor: t.panel, borderColor: t.border }]}
          onPress={() => router.push({ pathname: "/host/[id]", params: { id: h.hostId } })}
          onLongPress={() =>
            Alert.alert(`Forget ${h.hostName}?`, "You'll need to pair again to use it.", [
              { text: "Cancel", style: "cancel" },
              { text: "Forget", style: "destructive", onPress: () => removeHost(h.hostId) },
            ])
          }
        >
          <Text style={[styles.rowTitle, { color: t.text }]}>{h.hostName}</Text>
          <Text style={{ color: t.muted }}>{h.lastAddress ?? h.addresses[0]}</Text>
        </Pressable>
      ))}
      <Pressable style={[styles.secondary, { borderColor: t.border }]} onPress={() => router.push("/pair")}>
        <Text style={{ color: t.text }}>Pair another computer</Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  empty: { flex: 1, padding: 24, justifyContent: "center", gap: 16 },
  title: { fontSize: 26, fontWeight: "700" },
  body: { fontSize: 16, lineHeight: 22 },
  primary: { padding: 14, borderRadius: 12, alignItems: "center" },
  primaryText: { fontSize: 16, fontWeight: "600" },
  list: { padding: 16, gap: 12 },
  row: { padding: 16, borderRadius: 12, borderWidth: 1, gap: 4 },
  rowTitle: { fontSize: 17, fontWeight: "600" },
  secondary: { padding: 14, borderRadius: 12, borderWidth: 1, alignItems: "center" },
});
