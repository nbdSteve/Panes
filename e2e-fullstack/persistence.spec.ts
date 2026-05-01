import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Persistence", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("workspace survives page reload", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "PersistWS");

    // Workspace should be in sidebar
    await expect(page.locator(".sidebar-item", { hasText: "PersistWS" })).toBeVisible();

    // Reload the page
    await page.reload();
    await page.waitForLoadState("networkidle");

    // Workspace should still be there (loaded from SQLite)
    await expect(page.locator(".sidebar-item", { hasText: "PersistWS" })).toBeVisible({
      timeout: 10_000,
    });
  });

  test("completed thread survives page reload", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "PersistThread");

    await sendPrompt(page, "hello world");
    await waitForCompletion(page);
    await expect(page.locator(".completion-label-text")).toHaveText("Complete");

    // Reload
    await page.reload();
    await page.waitForLoadState("networkidle");

    // Click the workspace to see its threads
    await page.locator(".sidebar-item", { hasText: "PersistThread" }).click();

    // Click the thread in the thread list to load it
    await page.locator(".thread-list-item").first().click({ timeout: 10_000 });

    // The thread prompt should be visible (events are in-memory, but prompt persists)
    await expect(page.locator(".thread-prompt-text", { hasText: "hello world" })).toBeVisible({
      timeout: 10_000,
    });
  });

  test("thread prompt text persists across reload", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "PersistPrompt");

    await sendPrompt(page, "hello persistence test");
    await waitForCompletion(page);

    await page.reload();
    await page.waitForLoadState("networkidle");

    await page.locator(".sidebar-item", { hasText: "PersistPrompt" }).click();

    // Click the thread to load it
    await page.locator(".thread-list-item").first().click({ timeout: 10_000 });

    // The original prompt text should be visible in the thread view
    await expect(page.locator(".thread-prompt-text", { hasText: "hello persistence test" })).toBeVisible({
      timeout: 10_000,
    });
  });
});
