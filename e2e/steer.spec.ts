import { test, expect } from "@playwright/test";

test.describe("Gate — Steer", () => {
  test("steer button appears on gate card", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/steer-ws");
    await page.click("text=Add");

    // "dangerous" keyword triggers gate
    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");
    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    // All three buttons should be visible
    await expect(page.getByRole("button", { name: "Continue" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Steer" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Abort" })).toBeVisible();
  });

  test("clicking steer opens text input", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/steer-input");
    await page.click("text=Add");

    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");
    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.locator(".gate-actions button", { hasText: "Steer" }).click();
    await expect(page.locator('.gate-steer-input textarea')).toBeVisible();
    await expect(page.locator('.gate-steer-input textarea')).toBeFocused();
  });

  test("escape cancels steer mode", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/steer-esc");
    await page.click("text=Add");

    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");
    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.locator(".gate-actions button", { hasText: "Steer" }).click();
    await expect(page.locator('.gate-steer-input textarea')).toBeVisible();

    await page.locator('.gate-steer-input textarea').press("Escape");
    await expect(page.locator('.gate-steer-input textarea')).not.toBeVisible();
  });
});
