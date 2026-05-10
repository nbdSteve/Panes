import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

/**
 * Full-stack tests that exercise the real ACP adapter against a live kiro-cli
 * binary. Opt-in: guarded by `PANES_KIRO_CLI_PATH` so CI without the binary
 * installed skips the whole file.
 *
 * To run locally:
 *   PANES_KIRO_CLI_PATH=$(which kiro-cli) npm run test:e2e:fullstack -- kiro-cli-real
 *
 * These tests are deliberately minimal: Step 8 of the ACP plan lists them as
 * a smoke-test layer on top of the comprehensive fake-agent integration
 * tests in crates/panes-adapters/tests/acp_integration.rs. The goal is to
 * catch wire-format drift when kiro-cli ships a new release, not to
 * re-verify the adapter's state machine.
 */

const KIRO_ENABLED = !!process.env.PANES_KIRO_CLI_PATH;

test.describe("Full-Stack: kiro-cli (real binary)", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    test.skip(
      !KIRO_ENABLED,
      "kiro-cli not installed — set PANES_KIRO_CLI_PATH to run"
    );
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-kiro-"));
  });

  test("kiro-cli registers as a selectable adapter", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);

    // Open the adapter dropdown — first config trigger.
    await page.locator(".config-dropdown-trigger").first().click();

    const menu = page.locator(".config-dropdown-menu").first();
    await expect(
      menu.locator(".config-dropdown-item-label", { hasText: "kiro-cli" })
    ).toBeVisible({ timeout: 5_000 });
  });

  test("simple prompt via kiro-cli completes end-to-end", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);

    // Pick kiro-cli.
    await page.locator(".config-dropdown-trigger").first().click();
    await page
      .locator(".config-dropdown-item-label", { hasText: "kiro-cli" })
      .click();

    // A trivial prompt that any LLM should complete quickly.
    await sendPrompt(page, "respond with only the word pong, nothing else");

    await waitForCompletion(page, 60_000);
    await expect(page.locator(".completion-label-text")).toHaveText("Complete");
  });

  test("cancel during a live prompt terminates the backend", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);

    await page.locator(".config-dropdown-trigger").first().click();
    await page
      .locator(".config-dropdown-item-label", { hasText: "kiro-cli" })
      .click();

    // Ask for something the model will stream for a while.
    await sendPrompt(
      page,
      "count from 1 to 100 slowly, one number per line, no other text"
    );

    // Wait for streaming to start (we see at least one event card), then stop.
    await page.locator(".thread-event, .completion-card, .gate-card").first().waitFor({ timeout: 30_000 });
    await page.click(".btn-stop");

    // The thread should transition to a cancelled/interrupted state within a
    // reasonable window — not hang on the cancelled ACP process.
    await expect(page.locator("text=Cancelled").first()).toBeVisible({ timeout: 15_000 });
  });
});
