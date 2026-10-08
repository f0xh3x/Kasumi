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
    // Two pages: the overlay and the settings window.
    rollupOptions: { input: { main: "index.html", settings: "settings.html" } },
  },
});
