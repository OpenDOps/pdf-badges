import { reactRouter } from "@react-router/dev/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

import { foldFormPlugin } from "./fold-plugin";

export default defineConfig(({ mode }) => ({
  plugins: [foldFormPlugin(), tailwindcss(), mode === "test" ? null : reactRouter()],
  publicDir: "public",
  server: {
    port: 5174,
    strictPort: true,
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/flags": "http://127.0.0.1:8080",
    },
    watch: {
      ignored: ["**/public/dbenums/**"],
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./app/test-setup.ts"],
  },
}));
