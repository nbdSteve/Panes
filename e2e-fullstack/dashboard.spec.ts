import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion, waitForGate } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Dashboard", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("dashboard shows workspace after adding", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");

    // Dashboard is default view
    await expect(page.locator(".dashboard-view")).toBeVisible({ timeout: 10000 });

    await addWorkspace(page, wsPath, "DashWS");

    // Navigate to dashboard
    await page.locator(".sidebar-item", { hasText: "Dashboard" }).click();
    await expect(page.locator(".dashboard-card", { hasText: "DashWS" })).toBeVisible({ timeout: 10000 });
  });

  test("running thread shows on dashboard with cost", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");
    await addWorkspace(page, wsPath, "CostDash");
    await sendPrompt(page, "hello world");
    await waitForCompletion(page);

    // Navigate to dashboard
    await page.locator(".sidebar-item", { hasText: "Dashboard" }).click();
    await expect(page.locator(".dashboard-card", { hasText: "CostDash" })).toBeVisible({ timeout: 10000 });
  });

  test("gate approval from dashboard completes thread", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");
    await addWorkspace(page, wsPath, "GateDash");
    await sendPrompt(page, "do something dangerous");
    await waitForGate(page);

    // Navigate to dashboard
    await page.locator(".sidebar-item", { hasText: "Dashboard" }).click();

    const card = page.locator(".dashboard-card", { hasText: "GateDash" });
    await expect(card).toBeVisible({ timeout: 10000 });
    await expect(card.locator("text=gate")).toBeVisible({ timeout: 5000 });

    // Approve from dashboard
    await card.locator("button:has-text('Continue')").click();

    // Gate status should resolve
    await expect(card.locator("text=gate")).not.toBeVisible({ timeout: 10000 });
  });
});
