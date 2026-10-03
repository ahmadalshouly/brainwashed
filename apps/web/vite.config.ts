import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The host's gateway serves dist/ (see crates/gateway/src/web.rs). In dev,
// point BRAINWASHED_HOST at a running gateway, e.g. http://127.0.0.1:47860,
// and open the pairing link with this server's address instead.
const host = process.env.BRAINWASHED_HOST;

export default defineConfig({
  plugins: [react()],
  server: host ? { proxy: { "/pair": host, "/rpc": host, "/hello": host } } : undefined,
  build: { target: "es2022", outDir: "dist" },
});
