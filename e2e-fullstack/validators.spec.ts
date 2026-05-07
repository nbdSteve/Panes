import { test, expect, type Page } from "@playwright/test";
import { addWorkspace, sendPrompt } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

async function enableValidatorsFeature(page: Page) {
  await page.locator("button[title='Settings']").click();
  await page.locator(".settings-section h3", { hasText: "Features" }).waitFor({ timeout: 5_000 });
  const row = page.locator(".settings-row", { hasText: "Output Validators" });
  const input = row.locator("input[type='checkbox']");
  if (!(await input.isChecked())) {
    await row.locator(".toggle-slider").click();
  }
}

async function addCitationValidator(page: Page, wsName: string) {
  await page.locator(".sidebar-item", { hasText: wsName }).click();
  await page.locator(".sidebar-item", { hasText: "Validators" }).click();
  await page.locator(".validators-panel").waitFor({ timeout: 5_000 });
  await page.locator(".validators-add-option", { hasText: "Citation Check" }).click();
  await expect(page.locator(".validators-item")).toHaveCount(1, { timeout: 5_000 });
}

test.describe("Full-Stack: Validator Gate", () => {
  let wsPath: string;

  test.beforeEach(() => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("validator failure surfaces gate card with citation finding", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath, "ValWS");
    await enableValidatorsFeature(page);
    await addCitationValidator(page, "ValWS");

    // Back to the workspace and send a prompt whose completion references a path
    // that does not exist inside the temp workspace.
    await page.locator(".sidebar-item", { hasText: "ValWS" }).click();
    await sendPrompt(page, "validate my output");

    const gate = page.locator(".gate-card.gate-validator");
    await expect(gate).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Validator found issues")).toBeVisible();
    await expect(page.locator("text=citation")).toBeVisible();
    await expect(
      page.locator("text=referenced path does not exist: src/missing.rs"),
    ).toBeVisible();
    await expect(page.locator("button:has-text('Accept anyway')")).toBeVisible();
    await expect(page.locator("button:has-text('Reject output')")).toBeVisible();
  });
});
