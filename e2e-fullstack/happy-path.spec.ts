import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion, waitForText } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Happy Path", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("send prompt -> events stream -> completion card renders", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "hello world");

    await waitForText(page, "hello world");
    await waitForCompletion(page);

    await expect(page.locator(".completion-label-text")).toHaveText("Complete");
    await expect(page.getByRole("button", { name: /commit/i })).not.toBeVisible();
  });
});
