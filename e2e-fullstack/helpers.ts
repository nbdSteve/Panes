import type { Page } from "@playwright/test";

export async function addWorkspace(page: Page, path: string, name?: string) {
  await page.locator("text=Add workspace").waitFor({ timeout: 10_000 });
  await page.click("text=Add workspace");
  await page.fill('input[placeholder="/path/to/project"]', path);
  if (name) {
    await page.fill('input[placeholder="Display name (optional)"]', name);
  }
  await page.click("text=Add");
  await page.locator(".thread-list").waitFor({ timeout: 10_000 });
}

export async function sendPrompt(page: Page, prompt: string) {
  const textarea = page.locator("textarea");
  await textarea.fill(prompt);
  await textarea.press("Enter");
}

export async function waitForCompletion(page: Page, timeout = 15_000) {
  // Use `.first()` — strict mode is enabled by default and would fail if
  // more than one completion card is on the page (e.g. a follow-up or
  // queued second prompt that's already completed). The helper's contract
  // is "at least one completion card exists", so first-match is correct.
  await page.locator(".completion-card").first().waitFor({ timeout });
  // The Complete event is forwarded to the frontend before the backend
  // finishes updating `threads.status = 'completed'` in SQLite (see
  // session.rs:763 vs 801). A small settle window avoids a downstream
  // race where a query like `list_all_threads` runs against the still-
  // running row and the UI appears empty. 100ms is well under typical
  // human perception and comfortably above the observed write latency.
  await page.waitForTimeout(100);
}

export async function waitForGate(page: Page, timeout = 15_000) {
  await page.locator(".gate-card").first().waitFor({ timeout });
}

export async function waitForError(page: Page, timeout = 15_000) {
  await page.locator(".error-card").first().waitFor({ timeout });
}

export async function waitForText(page: Page, text: string, timeout = 10_000) {
  await page.getByText(text).first().waitFor({ timeout });
}
