import "../lib/random";
import { Stack } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { HostsProvider } from "../lib/hosts";
import { useTheme } from "../lib/theme";

export default function RootLayout() {
  const t = useTheme();
  return (
    <HostsProvider>
      <StatusBar style="auto" />
      <Stack
        screenOptions={{
          headerStyle: { backgroundColor: t.panel },
          headerTintColor: t.text,
          contentStyle: { backgroundColor: t.bg },
        }}
      >
        <Stack.Screen name="index" options={{ title: "BrainWashed" }} />
        <Stack.Screen name="pair" options={{ title: "Pair a computer", presentation: "modal" }} />
        <Stack.Screen name="host/[id]/index" options={{ title: "Chat" }} />
        <Stack.Screen name="host/[id]/models" options={{ title: "Models" }} />
        <Stack.Screen name="host/[id]/skills" options={{ title: "Skills" }} />
      </Stack>
    </HostsProvider>
  );
}
