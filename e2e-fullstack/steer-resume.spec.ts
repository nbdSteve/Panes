import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForGate, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Steer & Resume", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("gate -> steer with text -> thread resolves as steered", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await page.waitForTimeout(500);

    // Click Steer button
    await page.locator(".gate-actions button", { hasText: "Steer" }).click();
    await expect(page.locator(".gate-steer-input textarea")).toBeVisible();

    // Type steer instructions and submit
    await page.locator(".gate-steer-input textarea").fill("do it safely instead");
    await page.locator(".gate-steer-input textarea").press("Enter");

    // Gate card should show steered state
    await expect(page.locator(".gate-steered")).toBeVisible({ timeout: 5000 });
    await expect(page.locator("text=Steered")).toBeVisible();
  });

  test("gate -> steer -> follow-up prompt resumes thread", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await page.waitForTimeout(500);

    // Steer
    await page.locator(".gate-actions button", { hasText: "Steer" }).click();
    await page.locator(".gate-steer-input textarea").fill("use a safe approach");
    await page.locator(".gate-steer-input textarea").press("Enter");

    // Wait for the steered gate to resolve and thread to stop
    await expect(page.locator(".gate-steered")).toBeVisible({ timeout: 5000 });

    // Now send a follow-up prompt to resume
    await sendPrompt(page, "hello follow-up after steer");

    // Should get a new completion (the resumed thread with text-only response)
    await page.waitForTimeout(5000);
    const completions = await page.locator(".completion-card").count();
    expect(completions).toBeGreaterThanOrEqual(1);
  });

  test("gate -> abort -> gate shows rejected", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await page.waitForTimeout(500);

    await page.click("button:has-text('Abort')");
    await expect(page.locator("text=Aborted")).toBeVisible({ timeout: 5000 });
  });

  test("gate -> approve -> completion", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await page.waitForTimeout(500);

    await page.click("button:has-text('Continue')");
    await expect(page.locator("text=Continued")).toBeVisible({ timeout: 5000 });
    await waitForCompletion(page);
    await expect(page.locator(".completion-label-text")).toHaveText("Complete");
  });
});
