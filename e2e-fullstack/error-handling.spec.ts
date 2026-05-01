import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForError } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Error Handling", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("error scenario -> error card renders", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "trigger an error please");

    await waitForError(page);
    await expect(page.locator(".error-card")).toBeVisible();
  });
});
