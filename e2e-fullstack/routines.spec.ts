import { test, expect, type Page } from "@playwright/test";
import { addWorkspace } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

async function enableRoutinesFeature(page: Page) {
  await page.locator("button[title='Settings']").click();
  await page.locator(".settings-section h3", { hasText: "Features" }).waitFor({ timeout: 5_000 });
  const row = page.locator(".settings-row", { hasText: "Routines" });
  const slider = row.locator(".toggle-slider");
  const input = row.locator("input[type='checkbox']");
  if (!(await input.isChecked())) {
    await slider.click();
  }
}

async function navigateToRoutines(page: Page, wsName: string) {
  await page.locator(".sidebar-item", { hasText: wsName }).click();
  await page.locator(".sidebar-item", { hasText: "Routines" }).click();
  await page.locator(".routine-panel").waitFor({ timeout: 5_000 });
}

test.describe("Full-Stack: Routines", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("feature toggle shows and hides sidebar routines item", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "Toggle WS");

    // Routines sidebar item should NOT be visible by default (feature off)
    await expect(page.locator(".sidebar-item", { hasText: "Routines" })).not.toBeVisible();

    // Enable routines feature
    await page.locator("button[title='Settings']").click();
    await page.locator(".settings-section h3", { hasText: "Features" }).waitFor({ timeout: 5_000 });
    const row = page.locator(".settings-row", { hasText: "Routines" });
    await row.locator(".toggle-slider").click();

    // Go back to workspace — Routines item should appear
    await page.locator(".sidebar-item", { hasText: "Toggle WS" }).click();
    await expect(page.locator(".sidebar-item", { hasText: "Routines" })).toBeVisible();

    // Disable routines feature
    await page.locator("button[title='Settings']").click();
    await page.locator(".settings-section h3", { hasText: "Features" }).waitFor({ timeout: 5_000 });
    const row2 = page.locator(".settings-row", { hasText: "Routines" });
    await row2.locator(".toggle-slider").click();

    // Go back to workspace — Routines item should be gone
    await page.locator(".sidebar-item", { hasText: "Toggle WS" }).click();
    await expect(page.locator(".sidebar-item", { hasText: "Routines" })).not.toBeVisible();
  });

  test("create routine, toggle, and delete", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "CRUD WS");
    await enableRoutinesFeature(page);
    await navigateToRoutines(page, "CRUD WS");

    // Should see empty state
    await expect(page.locator(".routine-empty")).toBeVisible({ timeout: 5_000 });

    // Create a routine
    await page.locator("button", { hasText: "New Routine" }).click();
    await expect(page.locator(".routine-form h3")).toHaveText("New Routine");

    await page.locator("textarea.routine-prompt-input").fill("Run daily dependency check");
    await page.locator(".schedule-presets button", { hasText: "Daily midnight" }).click();
    await page.locator("button", { hasText: "Create Routine" }).click();

    // Should return to routine list with the new routine
    await expect(page.locator(".routine-item")).toHaveCount(1, { timeout: 5_000 });
    await expect(page.locator(".routine-prompt")).toContainText("Run daily dependency check");
    await expect(page.locator(".routine-schedule")).toContainText("Daily at 0:00");

    // Sidebar badge should show 1
    await expect(page.locator(".sidebar-item", { hasText: "Routines" }).locator(".thread-count")).toHaveText("1");

    // Toggle routine off via the visible slider
    const itemToggle = page.locator(".routine-item .toggle-slider");
    await itemToggle.click();

    // Sidebar badge should disappear (0 enabled)
    await expect(page.locator(".sidebar-item", { hasText: "Routines" }).locator(".thread-count")).not.toBeVisible();

    // Toggle back on
    await itemToggle.click();
    await expect(page.locator(".sidebar-item", { hasText: "Routines" }).locator(".thread-count")).toHaveText("1");

    // Delete routine (first click shows confirm, second deletes)
    await page.locator(".routine-item .btn-delete-inline").click();
    await expect(page.locator(".routine-item button.btn-danger", { hasText: "Confirm?" })).toBeVisible();
    await page.locator(".routine-item button.btn-danger", { hasText: "Confirm?" }).click();

    // Should return to empty state
    await expect(page.locator(".routine-empty")).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(".sidebar-item", { hasText: "Routines" }).locator(".thread-count")).not.toBeVisible();
  });

  test("routine form validation requires prompt", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "Valid WS");
    await enableRoutinesFeature(page);
    await navigateToRoutines(page, "Valid WS");

    await page.locator("button", { hasText: "New Routine" }).click();

    // Submit without prompt
    await page.locator("button", { hasText: "Create Routine" }).click();
    await expect(page.locator(".inline-error")).toContainText("Prompt is required");

    // Cancel returns to list
    await page.locator(".routine-form button", { hasText: "Cancel" }).click();
    await expect(page.locator(".routine-empty")).toBeVisible();
  });

  test("routine execution history expands and collapses", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "Hist WS");
    await enableRoutinesFeature(page);
    await navigateToRoutines(page, "Hist WS");

    // Create a routine
    await page.locator("button", { hasText: "New Routine" }).click();
    await page.locator("textarea.routine-prompt-input").fill("Check logs hourly");
    await page.locator(".schedule-presets button", { hasText: "Hourly" }).click();
    await page.locator("button", { hasText: "Create Routine" }).click();

    await expect(page.locator(".routine-item")).toHaveCount(1, { timeout: 5_000 });

    // Click to expand execution history
    await page.locator(".routine-item-header").click();
    await expect(page.locator(".routine-executions")).toBeVisible();
    await expect(page.locator(".routine-executions h4")).toHaveText("Execution History");
    await expect(page.locator(".routine-executions .muted")).toHaveText("No executions yet");

    // Click again to collapse
    await page.locator(".routine-item-header").click();
    await expect(page.locator(".routine-executions")).not.toBeVisible();
  });
});
