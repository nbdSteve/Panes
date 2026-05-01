import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e-fullstack",
  timeout: 30_000,
  retries: 0,
  workers: 1,
  globalSetup: "./e2e-fullstack/global-setup.ts",
  globalTeardown: "./e2e-fullstack/global-teardown.ts",
  use: {
    baseURL: "http://127.0.0.1:5174",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "webkit",
      use: { browserName: "webkit" },
    },
  ],
});
