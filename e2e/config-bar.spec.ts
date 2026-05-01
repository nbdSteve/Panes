import { test, expect } from "@playwright/test";

function setupWorkspace(page: any) {
  return (async () => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");
  })();
}

test.describe("Config Bar — Adapter Selector", () => {
  test("adapter dropdown appears and shows claude-code", async ({ page }) => {
    await setupWorkspace(page);
    const trigger = page.locator(".config-dropdown-trigger").first();
    await expect(trigger).toBeVisible();
    await expect(trigger).toContainText("claude-code");
  });

  test("adapter dropdown opens and closes on click", async ({ page }) => {
    await setupWorkspace(page);
    const trigger = page.locator(".config-dropdown-trigger").first();
    await trigger.click();
    await expect(page.locator(".config-dropdown-menu").first()).toBeVisible();
    await trigger.click();
    await expect(page.locator(".config-dropdown-menu")).not.toBeVisible();
  });
});

test.describe("Config Bar — Agent Selector", () => {
  test("agent dropdown shows Default and agent list", async ({ page }) => {
    await setupWorkspace(page);
    // Agent dropdown is the second config-dropdown-trigger
    const triggers = page.locator(".config-dropdown-trigger");
    await triggers.nth(1).click();

    const menu = page.locator(".config-dropdown-menu");
    await expect(menu).toBeVisible();

    // Should have "Default" option
    await expect(menu.locator("text=Default")).toBeVisible();
    await expect(menu.locator("text=No agent override")).toBeVisible();

    // Should have at least one named agent from the mock
    await expect(menu.locator("text=codebase-analyzer")).toBeVisible();
  });

  test("selecting an agent with a model auto-sets model dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Open agent dropdown
    await triggers.nth(1).click();

    // Select codebase-analyzer (model: sonnet)
    await page.click(".config-dropdown-item:has-text('codebase-analyzer')");

    // Agent trigger should now show the selected agent
    await expect(triggers.nth(1)).toContainText("codebase-analyzer");

    // Model trigger should show Sonnet
    await expect(triggers.nth(2)).toContainText("Sonnet");
  });

  test("selecting an agent with a model locks the model dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Select codebase-pattern-finder (model: opus)
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('codebase-pattern-finder')");

    // Model should show Opus
    await expect(triggers.nth(2)).toContainText("Opus");

    // Model trigger should be locked (disabled)
    await expect(triggers.nth(2)).toBeDisabled();

    // Clicking locked model should not open menu
    await triggers.nth(2).click({ force: true });
    await expect(page.locator(".config-dropdown-menu")).not.toBeVisible();
  });

  test("selecting Default agent unlocks model dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // First lock it by selecting an agent with a model
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('codebase-analyzer')");
    await expect(triggers.nth(2)).toBeDisabled();

    // Now select Default
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('Default')");

    // Model should be unlocked
    await expect(triggers.nth(2)).not.toBeDisabled();
  });

  test("agent without a model does not lock model dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Select karen (no model defined)
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('karen')");

    // Model should remain unlocked
    await expect(triggers.nth(2)).not.toBeDisabled();
  });

  test("agents show model badges in dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    await triggers.nth(1).click();
    const menu = page.locator(".config-dropdown-menu");

    // Agents with models should show badges
    await expect(menu.locator(".config-dropdown-item-badge").first()).toBeVisible();
  });
});

test.describe("Config Bar — Model Selector", () => {
  test("model dropdown shows Sonnet, Opus, Haiku", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");
    await triggers.nth(2).click();

    const menu = page.locator(".config-dropdown-menu");
    await expect(menu.locator("text=Sonnet")).toBeVisible();
    await expect(menu.locator("text=Opus")).toBeVisible();
    await expect(menu.locator("text=Haiku")).toBeVisible();
  });

  test("selecting a model updates the trigger text", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    await triggers.nth(2).click();
    await page.click(".config-dropdown-item:has-text('Opus')");

    await expect(triggers.nth(2)).toContainText("Opus");
  });

  test("model descriptions are shown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");
    await triggers.nth(2).click();

    await expect(page.locator("text=Fast & capable")).toBeVisible();
    await expect(page.locator("text=Most capable")).toBeVisible();
    await expect(page.locator("text=Fastest")).toBeVisible();
  });
});

test.describe("Config Bar — Agent Selection Sends Correctly", () => {
  test("prompt with Default agent completes without error", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Ensure agent is "Default" (the initial state)
    await expect(triggers.nth(1)).toContainText("Default");

    // Send a prompt — should succeed, not error with "unknown agent"
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".error-card")).not.toBeVisible();
  });

  test("prompt after selecting and deselecting an agent completes without error", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Select a named agent
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('karen')");
    await expect(triggers.nth(1)).toContainText("karen");

    // Switch back to Default
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('Default')");
    await expect(triggers.nth(1)).toContainText("Default");

    // Send a prompt — should succeed
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".error-card")).not.toBeVisible();
  });
});

test.describe("Config Bar — Negative Cases", () => {
  test("dropdowns are disabled while thread is running", async ({ page }) => {
    await setupWorkspace(page);

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Wait for running state
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });

    // All config triggers should be disabled
    const triggers = page.locator(".config-dropdown-trigger");
    const count = await triggers.count();
    for (let i = 0; i < count; i++) {
      await expect(triggers.nth(i)).toBeDisabled();
    }
  });

  test("clicking outside closes dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Open agent dropdown
    await triggers.nth(1).click();
    await expect(page.locator(".config-dropdown-menu")).toBeVisible();

    // Click outside — use the thread header which is always visible and above the dropdown
    await page.click(".thread-header");
    await expect(page.locator(".config-dropdown-menu")).not.toBeVisible();
  });

  test("opening one dropdown closes another", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Open adapter dropdown
    await triggers.nth(0).click();
    await expect(page.locator(".config-dropdown-menu")).toBeVisible();

    // Open agent dropdown — adapter should close
    await triggers.nth(1).click();
    const menus = page.locator(".config-dropdown-menu");
    await expect(menus).toHaveCount(1);
  });

  test("config bar selections persist while composing a message", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Select an agent
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('karen')");

    // Select Opus model
    await triggers.nth(2).click();
    await page.click(".config-dropdown-item:has-text('Opus')");

    // Type in the textarea without sending
    await page.fill("textarea", "some draft message");

    // Selections should still be there
    await expect(triggers.nth(1)).toContainText("karen");
    await expect(triggers.nth(2)).toContainText("Opus");
  });
});

test.describe("Config Bar — Config Persistence", () => {
  async function addWorkspace(page: any, path: string, name: string) {
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', path);
    await page.fill('input[placeholder="Display name (optional)"]', name);
    await page.click("text=Add");
    await page.waitForSelector(".thread-list", { timeout: 5000 });
  }

  test("config persists when switching between workspaces", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, "/tmp/ws-persist-a", "Alpha");

    const triggers = page.locator(".config-dropdown-trigger");

    // Set Alpha to karen / Opus
    await triggers.nth(1).click();
    await page.click('.config-dropdown-item:has-text("karen")');
    await triggers.nth(2).click();
    await page.click('.config-dropdown-item:has-text("Opus")');

    // Add second workspace
    await addWorkspace(page, "/tmp/ws-persist-b", "Beta");

    // Change Beta to Default / Haiku
    await triggers.nth(1).click();
    await page.click('.config-dropdown-item:has-text("Default")');
    await triggers.nth(2).click();
    await page.click('.config-dropdown-item:has-text("Haiku")');

    // Switch back to Alpha — should restore karen / Opus
    await page.click('.sidebar-item:has-text("Alpha")');
    await expect(triggers.nth(1)).toContainText("karen");
    await expect(triggers.nth(2)).toContainText("Opus");

    // Switch back to Beta — should restore Default / Haiku
    await page.click('.sidebar-item:has-text("Beta")');
    await expect(triggers.nth(1)).toContainText("Default");
    await expect(triggers.nth(2)).toContainText("Haiku");
  });

  test("new workspace inherits most recently used config", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, "/tmp/ws-inherit-a", "First");

    const triggers = page.locator(".config-dropdown-trigger");

    // Change config to karen / Opus
    await triggers.nth(1).click();
    await page.click('.config-dropdown-item:has-text("karen")');
    await triggers.nth(2).click();
    await page.click('.config-dropdown-item:has-text("Opus")');

    // Add new workspace — should inherit karen / Opus
    await addWorkspace(page, "/tmp/ws-inherit-b", "Second");
    await expect(triggers.nth(1)).toContainText("karen");
    await expect(triggers.nth(2)).toContainText("Opus");
  });

  test("config persists across new threads in same workspace", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, "/tmp/ws-thread-persist", "Persist");

    const triggers = page.locator(".config-dropdown-trigger");

    // Set to karen / Opus then switch to Default so prompt succeeds
    await triggers.nth(2).click();
    await page.click('.config-dropdown-item:has-text("Opus")');

    // Send a prompt to create a thread
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await page.locator(".completion-card").waitFor({ timeout: 5000 });

    // Set agent to karen after completion
    await triggers.nth(1).click();
    await page.click('.config-dropdown-item:has-text("karen")');

    // Start new thread
    await page.click(".thread-list-new");
    await page.waitForTimeout(300);

    // Config should persist
    await expect(triggers.nth(1)).toContainText("karen");
    await expect(triggers.nth(2)).toContainText("Opus");
  });
});

test.describe("Config Bar — Disabled Tooltips", () => {
  test("config dropdowns show tooltip when disabled during run", async ({ page }) => {
    await setupWorkspace(page);

    await page.fill("textarea", "do something slow and complex");
    await page.press("textarea", "Enter");
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });

    const triggers = page.locator(".config-dropdown-trigger");
    const count = await triggers.count();
    for (let i = 0; i < count; i++) {
      await expect(triggers.nth(i)).toHaveAttribute("title", "Cannot change while thread is running");
    }
  });

  test("send button shows tooltip when empty", async ({ page }) => {
    await setupWorkspace(page);
    const sendBtn = page.locator(".btn-send");
    await expect(sendBtn).toBeDisabled();
    await expect(sendBtn).toHaveAttribute("title", "Enter a message to send");
  });

  test("send button tooltip changes when text is entered", async ({ page }) => {
    await setupWorkspace(page);
    await page.fill("textarea", "hello");
    const sendBtn = page.locator(".btn-send");
    await expect(sendBtn).toHaveAttribute("title", "Send (Enter)");
  });

  test("model dropdown shows locked tooltip when agent sets model", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    // Select an agent with a model (e.g. codebase-analyzer has model: sonnet)
    await triggers.nth(1).click();
    await page.click(".config-dropdown-item:has-text('codebase-analyzer')");

    await expect(triggers.nth(2)).toHaveAttribute("title", "Set by codebase-analyzer agent");
  });
});

test.describe("Config Bar — Dynamic Models", () => {
  test("model dropdown shows models from backend", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    await triggers.nth(2).click();
    const menu = page.locator(".config-dropdown-menu");
    await expect(menu.locator("text=Sonnet")).toBeVisible();
    await expect(menu.locator("text=Opus")).toBeVisible();
    await expect(menu.locator("text=Haiku")).toBeVisible();
  });

  test("model descriptions are shown in dropdown", async ({ page }) => {
    await setupWorkspace(page);
    const triggers = page.locator(".config-dropdown-trigger");

    await triggers.nth(2).click();
    const menu = page.locator(".config-dropdown-menu");
    await expect(menu.locator("text=Fast & capable")).toBeVisible();
    await expect(menu.locator("text=Most capable")).toBeVisible();
    await expect(menu.locator("text=Fastest")).toBeVisible();
  });
});
