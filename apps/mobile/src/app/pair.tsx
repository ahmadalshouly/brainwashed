import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { CameraView, useCameraPermissions } from "expo-camera";
import * as Device from "expo-device";
import { router, useLocalSearchParams } from "expo-router";
import { pairWithHost, parsePairingUrl } from "@brainwashed/api";
import { remoteFetch, useHosts } from "../lib/hosts";
import { useTheme } from "../lib/theme";

/** Rebuilds the pairing link when the app was opened from it by the system camera. */
function linkFromParams(params: Record<string, string | string[]>): string | null {
  if (!params.t || !params.k) return null;
  const q = Object.entries(params)
    .map(([k, v]) => `${k}=${encodeURIComponent(Array.isArray(v) ? v[0] : v)}`)
    .join("&");
  return `brainwashed://pair?${q}`;
}

export default function Pair() {
  const t = useTheme();
  const params = useLocalSearchParams();
  const { addHost } = useHosts();
  const [permission, requestPermission] = useCameraPermissions();
  const [manual, setManual] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const handled = useRef(false);

  const pair = async (link: string) => {
    if (handled.current) return;
    handled.current = true;
    setBusy(true);
    setError(null);
    try {
      const info = parsePairingUrl(link);
      const name = Device.deviceName ?? Device.modelName ?? "Phone";
      const host = await pairWithHost(info, name, remoteFetch);
      await addHost(host);
      router.replace({ pathname: "/host/[id]", params: { id: host.hostId } });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      handled.current = false;
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    const link = linkFromParams(params as Record<string, string>);
    if (link) pair(link);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const canScan = Platform.OS !== "web" && permission?.granted;

  return (
    <View style={styles.container}>
      {busy ? (
        <View style={styles.center}>
          <ActivityIndicator />
          <Text style={{ color: t.muted }}>Pairing…</Text>
        </View>
      ) : canScan ? (
        <CameraView
          style={styles.camera}
          barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
          onBarcodeScanned={({ data }) => {
            if (data.startsWith("brainwashed://pair")) pair(data);
          }}
        />
      ) : Platform.OS !== "web" ? (
        <Pressable style={[styles.primary, { backgroundColor: t.accent }]} onPress={requestPermission}>
          <Text style={{ color: t.accentText, fontWeight: "600" }}>Allow camera to scan the code</Text>
        </Pressable>
      ) : null}

      {error && <Text style={[styles.error, { color: t.danger }]}>{error}</Text>}

      <Text style={{ color: t.muted }}>Or paste the pairing link:</Text>
      <TextInput
        value={manual}
        onChangeText={setManual}
        placeholder="brainwashed://pair?…"
        placeholderTextColor={t.muted}
        autoCapitalize="none"
        autoCorrect={false}
        style={[styles.input, { color: t.text, borderColor: t.border, backgroundColor: t.panel }]}
      />
      <Pressable
        disabled={!manual.trim() || busy}
        style={[styles.secondary, { borderColor: t.border, opacity: manual.trim() ? 1 : 0.5 }]}
        onPress={() => pair(manual.trim())}
      >
        <Text style={{ color: t.text }}>Pair</Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, padding: 16, gap: 12 },
  center: { alignItems: "center", gap: 8, paddingVertical: 48 },
  camera: { width: "100%", aspectRatio: 1, borderRadius: 16, overflow: "hidden" },
  primary: { padding: 14, borderRadius: 12, alignItems: "center" },
  secondary: { padding: 12, borderRadius: 12, borderWidth: 1, alignItems: "center" },
  input: { borderWidth: 1, borderRadius: 10, padding: 12, fontSize: 14 },
  error: { fontSize: 15 },
});
