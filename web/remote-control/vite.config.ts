import { readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";

import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { VitePWA } from "vite-plugin-pwa";
import { defineConfig } from "vitest/config";

function clinchIcons(): Plugin {
  const icon = (size: 256 | 512) =>
    fileURLToPath(
      new URL(`../../app/channels/stable/icon/no-padding/${size}x${size}.png`, import.meta.url),
    );
  return {
    name: "clinch-remote-control-icons",
    generateBundle() {
      for (const size of [256, 512] as const) {
        this.emitFile({
          type: "asset",
          fileName: `icons/clinch-${size}.png`,
          source: readFileSync(icon(size)),
        });
      }
    },
  };
}

export default defineConfig({
  base: "./",
  plugins: [
    react(),
    clinchIcons(),
    VitePWA({
      strategies: "injectManifest",
      srcDir: "src",
      filename: "sw.ts",
      registerType: "autoUpdate",
      manifest: false,
      injectManifest: {
        globPatterns: ["**/*.{html,js,css,png,svg,webmanifest,woff2}"],
      },
    }),
  ],
  build: {
    sourcemap: false,
    target: "es2022",
  },
  test: {
    environment: "jsdom",
    setupFiles: [],
    exclude: ["tests/e2e/**", "**/node_modules/**", "**/dist/**"],
  },
});
