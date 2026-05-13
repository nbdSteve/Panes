import { test, expect } from "@playwright/test";

/**
 * Phase 2 Merge button — mock E2E.
 *
 * The tauriMock's start_thread returns worktreeStatus: "isolated" for
 * workspaces whose path begins with "/tmp/git-", which is how this test
 * exercises the Merge / Discard UI without needing a real git repo.
 * merge_to_main in the mock always returns fast_forwarded, so we're
 * testing the wiring (button -> handler -> UI transition), not the
 * merge engine itself (that has real-git tests in crates/panes-core).
 */
async function addGitWorkspaceAndSend(page: any, prompt: string) {
  await page.goto("/");
  await page.click("text=Add workspace");
  await page.fill('input[placeholder="/path/to/project"]', "/tmp/git-test-ws");
  await page.click("text=Add");
  await page.fill("textarea", prompt);
  await page.press("textarea", "Enter");
}

test.describe("Phase 2 worktree — Merge button", () => {
  test("completion card renders 'Merge to main' and 'Discard worktree' for isolated threads", async ({ page }) => {
    await addGitWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Merge is present, Revert label swaps to Discard.
    await expect(page.locator("button:has-text('Merge to main')")).toBeVisible();
    await expect(page.locator("button:has-text('Discard worktree')")).toBeVisible();
    await expect(page.locator("button:has-text('Revert all')")).not.toBeVisible();
  });

  test("clicking Merge transitions the card into the committed state", async ({ page }) => {
    await addGitWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Merge to main')");

    // Mock returns fast_forwarded — frontend maps that to the committed
    // action badge so the card collapses into its post-action state.
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 2000 });
    await expect(page.locator("button:has-text('Merge to main')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Discard worktree')")).not.toBeVisible();
  });

  test("non-git workspace still shows the Phase 1 Revert / Keep UI", async ({ page }) => {
    // Counter-check: the mock only marks workspaces under /tmp/git-* as
    // isolated, so the default /tmp/test-ws path should keep the old
    // labels. This guards against accidentally flipping every workspace
    // into worktree mode.
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");
    await page.fill("textarea", "edit the files");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator("button:has-text('Revert all')")).toBeVisible();
    await expect(page.locator("button:has-text('Merge to main')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Discard worktree')")).not.toBeVisible();
  });
});
