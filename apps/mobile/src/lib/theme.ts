import { useColorScheme } from "react-native";

const light = {
  bg: "#f7f7f8",
  panel: "#ffffff",
  text: "#1b1b1f",
  muted: "#6b6b76",
  border: "#e3e3e8",
  accent: "#4f46e5",
  accentText: "#ffffff",
  user: "#eef2ff",
  danger: "#c62828",
  ok: "#2e7d32",
};

const dark: typeof light = {
  bg: "#18181b",
  panel: "#222226",
  text: "#ececf1",
  muted: "#9a9aa6",
  border: "#34343a",
  accent: "#6366f1",
  accentText: "#ffffff",
  user: "#2b2a4a",
  danger: "#ef5350",
  ok: "#66bb6a",
};

export type Theme = typeof light;

export function useTheme(): Theme {
  return useColorScheme() === "dark" ? dark : light;
}
