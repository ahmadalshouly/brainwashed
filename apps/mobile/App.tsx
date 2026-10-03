import { StatusBar } from "expo-status-bar";
import { StyleSheet, Text, View, useColorScheme } from "react-native";
import type { HostInfo } from "@brainwashed/api";

// Placeholder until pairing lands in Phase 3.
const host: HostInfo | null = null;

export default function App() {
  const dark = useColorScheme() === "dark";
  const color = dark ? "#ececf1" : "#1b1b1f";

  return (
    <View style={[styles.container, { backgroundColor: dark ? "#18181b" : "#f7f7f8" }]}>
      <Text style={[styles.title, { color }]}>BrainWashed</Text>
      <Text style={[styles.subtitle, { color }]}>
        {host ? `Connected to ${host.name}` : "No host paired yet"}
      </Text>
      <StatusBar style="auto" />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: 16,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
  },
  subtitle: {
    marginTop: 8,
    opacity: 0.7,
  },
});
