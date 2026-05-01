import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Cost Tracking", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("completion card shows cost", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "hello world");

    await waitForCompletion(page);
    const card = page.locator(".completion-card");
    await expect(card).toBeVisible();

    const text = await card.textContent();
    expect(text).toMatch(/\$/);
  });
});
