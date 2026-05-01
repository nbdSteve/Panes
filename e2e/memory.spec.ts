import { test, expect } from "@playwright/test";

let wsCounter = 0;

async function addWorkspace(page: import("@playwright/test").Page, name?: string) {
  wsCounter++;
  const wsName = name ?? `MemTest${wsCounter}`;
  await page.goto("/");
  await page.click("text=Add workspace");
  await page.fill('input[placeholder="/path/to/project"]', `/tmp/test-mem-${wsCounter}`);
  await page.fill('input[placeholder="Display name (optional)"]', wsName);
  await page.click(".sidebar-footer button:has-text('Add')");
  await expect(page.locator(".sidebar-item", { hasText: wsName })).toBeVisible();
  return wsName;
}

async function completeThread(page: import("@playwright/test").Page, prompt: string) {
  await page.fill("textarea", prompt);
  await page.press("textarea", "Enter");
  await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
}

async function openMemoryPanel(page: import("@playwright/test").Page) {
  await page.click(".sidebar-item:has-text('Memory')");
  await expect(page.locator(".memory-panel")).toBeVisible();
}

test.describe("Memory Panel", () => {
  test("Memory nav item appears when workspace is selected", async ({ page }) => {
    await addWorkspace(page);
    await expect(page.locator(".sidebar-item", { hasText: "Memory" })).toBeVisible();
  });

  test("Memory nav item is not visible without a workspace", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".sidebar-item", { hasText: "Memory" })).not.toBeVisible();
  });

  test("opens memory panel with empty state", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);

    await expect(page.locator("text=No memories yet")).toBeVisible();
    await expect(page.locator("text=No briefing set")).toBeVisible();
  });

  test("memory panel highlights in sidebar when active", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);
    await expect(page.locator(".sidebar-item.active", { hasText: "Memory" })).toBeVisible();
  });
});

test.describe("Briefing CRUD", () => {
  test("can add a briefing", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);

    await page.locator(".memory-panel button:has-text('Add')").click();
    await expect(page.locator(".briefing-textarea")).toBeVisible();

    await page.fill(".briefing-textarea", "Always use TypeScript strict mode");
    await page.locator(".briefing-actions button:has-text('Save')").click();

    await expect(page.locator(".briefing-content")).toHaveText("Always use TypeScript strict mode");
  });

  test("can edit an existing briefing", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);

    await page.locator(".memory-panel button:has-text('Add')").click();
    await page.fill(".briefing-textarea", "Original briefing");
    await page.locator(".briefing-actions button:has-text('Save')").click();

    await page.locator(".memory-panel button:has-text('Edit')").click();
    await page.fill(".briefing-textarea", "Updated briefing content");
    await page.locator(".briefing-actions button:has-text('Save')").click();

    await expect(page.locator(".briefing-content")).toHaveText("Updated briefing content");
  });

  test("can cancel briefing edit", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);

    await page.locator(".memory-panel button:has-text('Add')").click();
    await page.fill(".briefing-textarea", "Will be cancelled");
    await page.locator(".briefing-actions button:has-text('Cancel')").click();

    await expect(page.locator("text=No briefing set")).toBeVisible();
  });

  test("deleting briefing by saving empty content", async ({ page }) => {
    await addWorkspace(page);
    await openMemoryPanel(page);

    await page.locator(".memory-panel button:has-text('Add')").click();
    await page.fill(".briefing-textarea", "Temporary briefing");
    await page.locator(".briefing-actions button:has-text('Save')").click();
    await expect(page.locator(".briefing-content")).toBeVisible();

    await page.locator(".memory-panel button:has-text('Edit')").click();
    await page.fill(".briefing-textarea", "");
    await page.locator(".briefing-actions button:has-text('Save')").click();

    await expect(page.locator("text=No briefing set")).toBeVisible();
  });
});

test.describe("Memory Extraction & Display", () => {
  test("completing a thread extracts memories visible in panel", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "hello world");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".memory-type").first()).toBeVisible();
  });

  test("multiple threads accumulate memories", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "first task");

    await page.click(".thread-list-new");
    await completeThread(page, "second task");

    await openMemoryPanel(page);
    const count = await page.locator(".memory-card").count();
    expect(count).toBeGreaterThanOrEqual(2);
  });

  test("memory count badge reflects extracted memories", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "test task");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });
    const count = await page.locator(".memory-count").textContent();
    expect(Number(count)).toBeGreaterThanOrEqual(1);
  });
});

test.describe("Memory CRUD Operations", () => {
  test("can pin and unpin a memory", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "pin test");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });

    await page.locator("[title='Pin']").first().click();
    await expect(page.locator(".memory-group-label", { hasText: "Pinned" })).toBeVisible();
    await expect(page.locator(".memory-card.pinned")).toBeVisible();

    await page.locator("[title='Unpin']").first().click();
    await expect(page.locator(".memory-group-label", { hasText: "Pinned" })).not.toBeVisible();
  });

  test("can edit a memory inline", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "edit test");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });

    await page.locator("[title='Edit']").first().click();
    await expect(page.locator(".memory-textarea")).toBeVisible();

    await page.fill(".memory-textarea", "Manually edited memory");
    await page.locator(".memory-edit-actions button:has-text('Save')").click();

    await expect(page.locator(".memory-content").first()).toHaveText("Manually edited memory");
  });

  test("can cancel memory edit", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "cancel edit test");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });

    const original = await page.locator(".memory-content").first().textContent();
    await page.locator("[title='Edit']").first().click();
    await page.fill(".memory-textarea", "This should not persist");
    await page.locator(".memory-edit-actions button:has-text('Cancel')").click();

    await expect(page.locator(".memory-content").first()).toHaveText(original!);
  });

  test("can delete a memory", async ({ page }) => {
    await addWorkspace(page);
    await completeThread(page, "delete test");

    await openMemoryPanel(page);
    const initialCount = await page.locator(".memory-card").count();
    expect(initialCount).toBeGreaterThanOrEqual(1);

    await page.locator("[title='Delete']").first().click();
    await page.locator("button:has-text('Confirm?')").first().click();

    await expect(page.locator(".memory-card")).toHaveCount(initialCount - 1);
  });
});

test.describe("Memory Panel Navigation", () => {
  test("switching between workspace view and memory view", async ({ page }) => {
    const name = await addWorkspace(page);
    await completeThread(page, "nav test");

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 3000 });
    const memCount = await page.locator(".memory-card").count();

    await page.click(`.sidebar-item:has-text('${name}')`);
    await expect(page.locator(".thread-list")).toBeVisible();
    await expect(page.locator(".memory-panel")).not.toBeVisible();

    await openMemoryPanel(page);
    await expect(page.locator(".memory-card")).toHaveCount(memCount);
  });
});
