import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome120",
    outDir: "dist",
    // One page: the settings window (and the setup and update notes windows, its views).
    rollupOptions: { input: { settings: "settings.html" } },
  },
});
