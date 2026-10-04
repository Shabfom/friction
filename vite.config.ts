import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import tsconfigPaths from "vite-tsconfig-paths";

// Pure client-side Vite SPA. No server runtime, no SSR, no Nitro.
// Tauri loads the resulting static dist/ folder over file://.
export default defineConfig({
  plugins: [react(), tailwindcss(), tsconfigPaths({ projects: ["./tsconfig.json"] })],
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  // Tauri expects a fixed port for the dev server.
  server: {
    port: 1420,
    strictPort: true,
  },
});
