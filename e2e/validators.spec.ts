import { test, expect } from "@playwright/test";

async function addWorkspaceAndSend(page: any, prompt: string) {
  await page.goto("/");
  await page.click("text=Add workspace");
  await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
  await page.click("text=Add");
  await page.fill("textarea", prompt);
  await page.press("textarea", "Enter");
}

test.describe("Validators — gate flow", () => {
  test("validator failure shows gate card with findings", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    const gate = page.locator(".gate-card.gate-validator");
    await expect(gate).toBeVisible({ timeout: 3000 });

    await expect(page.locator("text=Validator found issues")).toBeVisible();
    await expect(page.locator("text=citation")).toBeVisible();
    await expect(
      page.locator("text=referenced path does not exist: src/missing.rs"),
    ).toBeVisible();
    await expect(page.locator("button:has-text('Accept anyway')")).toBeVisible();
    await expect(page.locator("button:has-text('Reject output')")).toBeVisible();
  });

  test("accept anyway resolves validator gate and thread completes", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click("button:has-text('Accept anyway')");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("reject output resolves validator gate and completes with rejection", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click("button:has-text('Reject output')");

    // The mock resumes with a "rejected" Complete. The gate card should no longer
    // be in its pending state.
    await expect(page.locator("button:has-text('Reject output')")).toHaveCount(0, {
      timeout: 5000,
    });
  });
});
