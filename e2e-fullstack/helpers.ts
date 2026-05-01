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
  await page.locator(".completion-card").waitFor({ timeout });
}

export async function waitForGate(page: Page, timeout = 15_000) {
  await page.locator(".gate-card").waitFor({ timeout });
}

export async function waitForError(page: Page, timeout = 15_000) {
  await page.locator(".error-card").waitFor({ timeout });
}

export async function waitForText(page: Page, text: string, timeout = 10_000) {
  await page.getByText(text).first().waitFor({ timeout });
}
