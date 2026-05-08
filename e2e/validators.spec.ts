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
    await expect(page.locator("button:has-text('Steer')")).toBeVisible();
    await expect(page.locator("button:has-text('Auto-fix')")).toBeVisible();
    await expect(page.locator("button:has-text('Reject')")).toBeVisible();
  });

  test("accept anyway resolves validator gate and thread completes", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click("button:has-text('Accept anyway')");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("auto-fix resumes the thread with a synthesized prompt", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click("button:has-text('Auto-fix')");

    // After auto-fix, the gate should resolve (steered) and a follow-up turn
    // runs; a second completion lands after the resume.
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 7000 });
    // The synthesized correction text should show up as the follow-up prompt.
    await expect(
      page.locator(".follow-up .thread-prompt-text", {
        hasText: "A validator flagged issues",
      }),
    ).toBeVisible({ timeout: 3000 });
  });

  test("steer pre-fills the textarea with the correction prompt and resumes on send", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click("button:has-text('Steer')");

    const textarea = page.locator(
      ".gate-validator textarea[placeholder='Describe the correction...']",
    );
    await expect(textarea).toBeVisible();
    await expect(textarea).toHaveValue(/A validator flagged issues/);

    await textarea.fill("actually use src/real.rs instead");
    await page.locator(".gate-validator .btn-steer-submit").click();

    await expect(
      page.locator(".follow-up .thread-prompt-text", {
        hasText: "actually use src/real.rs instead",
      }),
    ).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("reject stops the thread but leaves it resumable via the textarea", async ({ page }) => {
    await addWorkspaceAndSend(page, "validate this output");

    await expect(page.locator(".gate-card.gate-validator")).toBeVisible({
      timeout: 3000,
    });

    await page.click(".gate-validator button:has-text('Reject')");

    // Thread is interrupted via a non-recoverable error.
    await expect(page.locator(".error-card")).toBeVisible({ timeout: 5000 });

    // User types a fresh message — this continues the same thread.
    await page.fill("textarea", "try something different");
    await page.press("textarea", "Enter");

    await expect(
      page.locator(".follow-up .thread-prompt-text", {
        hasText: "try something different",
      }),
    ).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });
});
