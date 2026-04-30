import { test, expect } from "@playwright/test";

// Feed activity stream is not yet implemented — feed view is a placeholder.
// These tests are skipped until the Feed component is built.

test.describe("Feed — Activity Stream", () => {
  test.skip("completed threads appear in feed with workspace name and cost", async ({ page }) => {});
  test.skip("feed shows threads from multiple workspaces sorted by recency", async ({ page }) => {});
  test.skip("feed shows aggregate cost total", async ({ page }) => {});
  test.skip("clicking feed item navigates to that thread", async ({ page }) => {});
});
