import { test, expect } from "@playwright/test";

test.describe("Sub-Agent Rendering", () => {
  test("sub-agent events render inside tool group", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/subagent-ws");
    await page.click("text=Add");

    // "subagent" keyword triggers sub-agent event sequence
    await page.fill("textarea", "use a subagent for this");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    // Should have a tool group for the Task tool
    const group = page.locator(".tool-group").first();
    await expect(group).toBeVisible();
    await expect(group).toContainText("Task");

    // Expand it to see nested sub-agent info
    await group.locator(".tool-group-header").click();
    await expect(group.locator(".sub-agent-nested")).toBeVisible();
    await expect(group.locator(".sub-agent-nested")).toContainText("Sub-agent:");
  });

  test("sub-agent shows cost in expanded view", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/subagent-cost");
    await page.click("text=Add");

    await page.fill("textarea", "use a subagent for this");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    const group = page.locator(".tool-group").first();
    await group.locator(".tool-group-header").click();
    await expect(group.locator(".sub-agent-cost")).toBeVisible();
    await expect(group.locator(".sub-agent-cost")).toContainText("$");
  });
});
