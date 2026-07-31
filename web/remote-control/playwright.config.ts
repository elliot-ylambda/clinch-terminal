import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  use: {
    baseURL: "http://127.0.0.1:4178",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "yarn vite preview --host 127.0.0.1 --port 4178",
    port: 4178,
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: "iPhone", use: { ...devices["iPhone 15 Pro"] } },
    { name: "iPad", use: { ...devices["iPad Pro 11"] } },
  ],
});
