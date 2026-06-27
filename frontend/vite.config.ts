import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";
import { viteSingleFile } from "vite-plugin-singlefile";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "XINGSHU_VITE_");
  const apiTarget = env.XINGSHU_VITE_API_TARGET ?? "http://127.0.0.1:8000";
  return {
    root: "frontend",
    plugins: [vue(), viteSingleFile()],
    build: {
      outDir: "dist",
      emptyOutDir: true,
      assetsInlineLimit: 100_000_000,
      cssCodeSplit: false,
      modulePreload: false,
      target: "es2020"
    },
    server: {
      host: "127.0.0.1",
      port: 5173,
      strictPort: false,
      proxy: {
        "/health": apiTarget,
        "/api": apiTarget,
        "/ws": {
          target: apiTarget,
          ws: true
        }
      }
    }
  };
});
