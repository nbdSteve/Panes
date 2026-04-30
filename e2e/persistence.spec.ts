import { test, expect } from "@playwright/test";

// Persistence tests require a real Tauri backend — the tauriMock in-memory
// store does not survive page reloads. Skip until running against full app.

test.describe("Persistence — Survives Reload", () => {
  test.skip("workspaces persist across page reload", async ({ page }) => {});
  test.skip("completed threads persist across page reload", async ({ page }) => {});
  test.skip("thread count badge persists across reload", async ({ page }) => {});
});
