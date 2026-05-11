import { test, expect } from "@playwright/test";

async function addWorkspace(page: any, path = "/tmp/kiro-test-ws") {
  await page.goto("/");
  await page.click("text=Add workspace");
  await page.fill('input[placeholder="/path/to/project"]', path);
  await page.click("text=Add");
  await page.locator(".thread-list").waitFor();
}

async function openAdapterDropdown(page: any) {
  const triggers = page.locator(".config-dropdown-trigger");
  // The adapter dropdown is the first config trigger.
  await triggers.first().click();
  await page.locator(".config-dropdown-menu").first().waitFor();
}

test.describe("kiro-cli agent", () => {
  test("kiro-cli appears in the agent picker alongside claude-code", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);

    const menu = page.locator(".config-dropdown-menu").first();
    await expect(menu.locator(".config-dropdown-item-label", { hasText: "claude-code" })).toBeVisible();
    await expect(menu.locator(".config-dropdown-item-label", { hasText: "kiro-cli" })).toBeVisible();
  });

  test("string 'acp' never appears as a selectable adapter", async ({ page }) => {
    // Guardrail: the protocol name must stay an implementation detail. If
    // something upstream ever registers the adapter under "acp", this test
    // catches the regression before a user sees it.
    await addWorkspace(page);
    await openAdapterDropdown(page);

    const menu = page.locator(".config-dropdown-menu").first();
    await expect(menu.locator(".config-dropdown-item-label", { hasText: /^acp$/ })).toHaveCount(0);
  });

  test("selecting kiro-cli switches the adapter label", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();

    // The trigger value should now read "kiro-cli".
    const triggerValue = page.locator(".config-dropdown-value").first();
    await expect(triggerValue).toHaveText("kiro-cli");
  });

  test("kiro-cli exposes discovered modes in the agent picker", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();

    // Give the agent list a moment to refresh for the new adapter.
    await page.waitForTimeout(200);

    // Open the agent (second) dropdown.
    const agentTrigger = page.locator(".config-dropdown-trigger").nth(1);
    await agentTrigger.click();

    // Wait for the menu to show kiro-cli modes (not the stale claude-code list).
    // The mock returns neutral ids; production probes the real backend.
    await expect(page.locator(".config-dropdown-item-label", { hasText: "mode-a" })).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".config-dropdown-item-label", { hasText: "mode-b" })).toBeVisible();
  });

  test("sending a text prompt via kiro-cli produces a completion card", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();

    await page.fill("textarea", "say hello");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("gated prompt via kiro-cli shows a gate card with risk label", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();

    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 3000 });
  });

  test("prompt sent while kiro-cli is selected actually routes through kiro-cli", async ({ page }) => {
    // Regression: before the adapter-change hook in App.tsx existed, selecting
    // kiro-cli in the UI left every start_thread call going through
    // claude-code. This test asserts the backend received adapter="kiro-cli".
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();
    await page.waitForTimeout(150);

    await page.fill("textarea", "say hi");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    const lastAdapter = await page.evaluate(async () => {
      // @ts-expect-error — the mock exposes __TAURI_INTERNALS__
      return window.__TAURI_INTERNALS__.invoke("__test_last_start_thread_adapter");
    });
    expect(lastAdapter).toBe("kiro-cli");
  });

  test("prompt sent while claude-code is selected routes through claude-code", async ({ page }) => {
    // The other side of the regression — verify the existing default still works.
    await addWorkspace(page);
    // Don't change the adapter — default should be claude-code.
    await page.fill("textarea", "say hi");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    const lastAdapter = await page.evaluate(async () => {
      // @ts-expect-error
      return window.__TAURI_INTERNALS__.invoke("__test_last_start_thread_adapter");
    });
    expect(lastAdapter).toBe("claude-code");
  });

  test("approving a gate via kiro-cli lets the thread complete", async ({ page }) => {
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();

    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 3000 });
    await page.click("button:has-text('Continue')");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("follow-up prompt on a kiro-cli thread resumes via kiro-cli, not claude-code", async ({
    page,
  }) => {
    // Regression: resume_thread ignored the adapter the thread was spawned
    // with and defaulted to claude-code when the frontend didn't pass a
    // hint (the UI never does). A kiro-cli follow-up would silently get
    // routed through the Claude CLI and fail with "failed to resume session"
    // because kiro-cli's UUID session ids mean nothing to Claude.
    await addWorkspace(page);
    await openAdapterDropdown(page);
    await page.locator(".config-dropdown-item-label", { hasText: "kiro-cli" }).click();
    await page.waitForTimeout(150);

    // Round 1: start a kiro-cli thread and wait for completion.
    await page.fill("textarea", "say hi");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    const startAdapter = await page.evaluate(async () => {
      // @ts-expect-error — mock exposes __TAURI_INTERNALS__
      return window.__TAURI_INTERNALS__.invoke("__test_last_start_thread_adapter");
    });
    expect(startAdapter).toBe("kiro-cli");

    // Round 2: send a follow-up. Completion → handleSendPrompt routes to
    // handleResumeThread → resume_thread IPC.
    await page.fill("textarea", "and say it again");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toHaveCount(2, { timeout: 5000 });

    const resumeAdapter = await page.evaluate(async () => {
      // @ts-expect-error
      return window.__TAURI_INTERNALS__.invoke("__test_last_resume_thread_adapter");
    });
    expect(
      resumeAdapter,
      "kiro-cli thread must resume via kiro-cli, not default to claude-code",
    ).toBe("kiro-cli");
  });
});
