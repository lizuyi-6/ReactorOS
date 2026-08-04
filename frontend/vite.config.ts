import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";
import { viteSingleFile } from "vite-plugin-singlefile";
import AutoImport from "unplugin-auto-import/vite";
import Components from "unplugin-vue-components/vite";
import { ElementPlusResolver } from "unplugin-vue-components/resolvers";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "XINGSHU_VITE_");
  const apiTarget = env.XINGSHU_VITE_API_TARGET ?? "http://127.0.0.1:8000";
  return {
    root: "frontend",
    // Element Plus on-demand: AutoImport resolves imperative APIs (ElMessage)
    // and Components resolves template components (el-button/el-table/...) and
    // the v-loading directive, each with its own CSS. `dirs: []` keeps custom
    // components (EmptyState/HmiButton) on their explicit imports; `dts: false`
    // skips the generated .d.ts (vite build does not type-check).
    plugins: [
      vue(),
      AutoImport({ resolvers: [ElementPlusResolver()], dts: false }),
      Components({ resolvers: [ElementPlusResolver()], dirs: [], dts: false }),
      viteSingleFile()
    ],
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
