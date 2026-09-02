import { defineConfig } from "vite";
import { resolve } from "node:path";

// Tide frontend: two entry pages (widget + settings).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Tauri rebuilds write into src-tauri/target; watching it crashes Vite (EBUSY on locked DLLs).
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "chrome105",
    rollupOptions: {
      input: {
        index: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
  },
});
