import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createGitWorkspace, isClean, getHeadHash } from "./fixtures/git-workspace";
import { getDataDir } from "./fixtures/tauri-app";
import { readdirSync, existsSync, writeFileSync, readFileSync } from "fs";
import { execSync } from "child_process";
import { join } from "path";

/**
 * Phase 2 merge conflict path — fullstack invariant.
 *
 * The worktree module has unit tests that prove `merge_into_head`
 * returns `Conflicts { files }` and leaves main's HEAD untouched. This
 * test exercises the full UI path: agent edits in worktree → user
 * commits inside worktree → main diverges → clicking "Merge to main"
 * surfaces a conflict notice with the file list and the completion
 * card stays open so the user can pick Discard.
 */
test.describe("Full-Stack: Git Worktree Merge Conflict", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
    // Seed src/main.rs so the fake adapter's Edit tool has a file to
    // modify and the main branch has something to diverge on.
    execSync("mkdir -p src", { cwd: wsPath });
    writeFileSync(join(wsPath, "src", "main.rs"), "fn main() { println!(\"base\"); }\n");
    execSync("git add src/main.rs && git commit -q -m 'seed src/main.rs'", {
      cwd: wsPath,
    });
  });

  test("merge returns conflicts when main has diverged; UI shows file list, card stays", async ({ page }) => {
    const mainHeadBefore = getHeadHash(wsPath);

    // Snapshot pre-existing worktree dirs (leftovers from other tests)
    // so we can reliably identify the one created by THIS test.
    const worktreesRoot = join(getDataDir(), "worktrees");
    const before = new Set(existsSync(worktreesRoot) ? readdirSync(worktreesRoot) : []);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");
    await waitForCompletion(page);

    // Wait for a fresh worktree dir to appear (one not in `before`).
    await expect
      .poll(
        () => {
          if (!existsSync(worktreesRoot)) return 0;
          return readdirSync(worktreesRoot).filter((n) => !before.has(n)).length;
        },
        { timeout: 5_000 },
      )
      .toBeGreaterThanOrEqual(1);
    const worktreeDir = join(
      worktreesRoot,
      readdirSync(worktreesRoot).find((n) => !before.has(n))!,
    );

    // The fake adapter writes src/main.rs in the worktree after the
    // Edit event fires. Wait for it to be dirty.
    await expect.poll(() => isClean(worktreeDir), { timeout: 5_000 }).toBe(false);

    // Commit the worktree's edit so its branch diverges from main —
    // without a worktree-side commit, merge_into_head would see
    // UpToDate and wouldn't conflict.
    execSync(
      'git -c user.email=t@t -c user.name=t commit -q -am "worktree edit"',
      { cwd: worktreeDir },
    );

    // Now advance main with a conflicting change on the SAME file.
    // This is the key to producing a real merge conflict.
    writeFileSync(join(wsPath, "src", "main.rs"), "fn main() { println!(\"MAIN DIVERGENT\"); }\n");
    execSync(
      'git -c user.email=t@t -c user.name=t commit -q -am "main divergent"',
      { cwd: wsPath },
    );
    const mainHeadBeforeMerge = getHeadHash(wsPath);

    // Click Merge to main — the IPC call goes to the backend which
    // runs libgit2's merge_analysis + merge, detects conflicts, and
    // restores main's HEAD before returning.
    await page.click("button:has-text('Merge to main')");

    // Conflict block appears with the file listed.
    await expect(page.locator(".merge-conflict")).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(".merge-conflict")).toContainText(/conflicting file/);
    await expect(page.locator(".merge-conflict-files")).toContainText("src/main.rs");

    // Main repo HEAD is unchanged — the merge aborted cleanly.
    expect(getHeadHash(wsPath)).toBe(mainHeadBeforeMerge);
    expect(getHeadHash(wsPath)).not.toBe(mainHeadBefore);

    // Completion card stays with Merge + Discard actions so the user
    // can recover. Discard worktree button should still be there.
    await expect(page.locator("button:has-text('Discard worktree')")).toBeVisible();
    await expect(page.locator("button:has-text('Merge to main')")).toBeVisible();

    // Discard cleans up the worktree dir.
    const worktreesBefore = readdirSync(worktreesRoot).length;
    await page.click("button:has-text('Discard worktree')");
    const confirmBtn = page.locator(".revert-confirm button:has-text('Revert')");
    await confirmBtn.waitFor({ timeout: 3_000 });
    await confirmBtn.click();
    await expect(page.locator("text=Reverted")).toBeVisible({ timeout: 5_000 });
    await expect
      .poll(() => readdirSync(worktreesRoot).length, { timeout: 5_000 })
      .toBeLessThan(worktreesBefore);
  });

  test("Option A resolution: 'Use yours' takes the worktree version and completes the merge", async ({ page }) => {
    // Same conflict setup, but instead of Discard, the user clicks
    // "Use yours" to resolve with the worktree's version. The merge
    // commits, main advances, and src/main.rs ends up with the
    // worktree content.
    const mainHeadBefore = getHeadHash(wsPath);
    const worktreesRoot = join(getDataDir(), "worktrees");
    const before = new Set(existsSync(worktreesRoot) ? readdirSync(worktreesRoot) : []);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");
    await waitForCompletion(page);

    await expect
      .poll(
        () => {
          if (!existsSync(worktreesRoot)) return 0;
          return readdirSync(worktreesRoot).filter((n) => !before.has(n)).length;
        },
        { timeout: 5_000 },
      )
      .toBeGreaterThanOrEqual(1);
    const worktreeDir = join(
      worktreesRoot,
      readdirSync(worktreesRoot).find((n) => !before.has(n))!,
    );

    await expect.poll(() => isClean(worktreeDir), { timeout: 5_000 }).toBe(false);

    execSync(
      'git -c user.email=t@t -c user.name=t commit -q -am "worktree edit"',
      { cwd: worktreeDir },
    );
    const worktreeContent = readFileSync(
      join(worktreeDir, "src", "main.rs"),
      "utf8",
    );

    writeFileSync(join(wsPath, "src", "main.rs"), "fn main() { println!(\"MAIN DIVERGENT\"); }\n");
    execSync(
      'git -c user.email=t@t -c user.name=t commit -q -am "main divergent"',
      { cwd: wsPath },
    );

    // Trigger the conflict.
    await page.click("button:has-text('Merge to main')");
    await expect(page.locator(".merge-conflict")).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(".merge-conflict-files")).toContainText("src/main.rs");

    // Pick "Use yours" to take the worktree version.
    await page.click(".merge-conflict-actions button:has-text('Use yours')");

    // Completion card transitions to committed and THIS test's worktree
    // dir is removed (other tests may have left their own worktrees
    // behind in the shared data dir).
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 5_000 });
    await expect
      .poll(() => existsSync(worktreeDir), { timeout: 5_000 })
      .toBe(false);

    // Main HEAD advanced with a merge commit (2 parents) and the file
    // now holds the worktree's content.
    const postHead = getHeadHash(wsPath);
    expect(postHead).not.toBe(mainHeadBefore);
    expect(readFileSync(join(wsPath, "src", "main.rs"), "utf8")).toBe(
      worktreeContent,
    );
    const parentCount = execSync(
      `git cat-file -p ${postHead} | grep -c "^parent "`,
      { cwd: wsPath, shell: "/bin/bash" },
    )
      .toString()
      .trim();
    expect(parentCount).toBe("2");
  });
});
