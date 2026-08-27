import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  build: {
    rollupOptions: {
      output: {
        // The framework/runtime is stable across releases — keep it in its own
        // chunk so app code changes never invalidate it and the WebView can
        // cache/parse it independently of route chunks. (Function form is
        // required — Vite 8 bundles Rolldown, which rejects the object form.)
        manualChunks(id) {
          if (/[\\/]node_modules[\\/](vue|vue-router|vue-i18n|@vue)[\\/]/.test(id)) {
            return "vue";
          }
          return undefined;
        },
      },
    },
    // Budget guard: a chunk growing past this warns at build time so bloat is
    // caught before it ships inside the desktop bundle.
    chunkSizeWarningLimit: 500,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
