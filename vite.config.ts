import { defineConfig } from "vite";

// Tauri erwartet ein statisches Frontend in `dist/`. Kein Framework — schlanke
// Vanilla-TS-UI (nelly-Stil: kleines Bundle, nur Status + Config).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
  },
});
