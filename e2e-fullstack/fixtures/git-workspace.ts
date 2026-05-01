import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

export function createGitWorkspace(): string {
  const dir = mkdtempSync(resolve(tmpdir(), "panes-git-ws-"));

  execSync("git init", { cwd: dir });
  execSync('git config user.email "test@panes.dev"', { cwd: dir });
  execSync('git config user.name "Panes Test"', { cwd: dir });

  writeFileSync(resolve(dir, "README.md"), "# Test Project\n");
  execSync("git add -A && git commit -m 'Initial commit'", { cwd: dir });

  return dir;
}

export function getHeadHash(dir: string): string {
  return execSync("git rev-parse HEAD", { cwd: dir }).toString().trim();
}

export function isClean(dir: string): boolean {
  return execSync("git status --porcelain", { cwd: dir }).toString().trim() === "";
}

export function lastCommitMessage(dir: string): string {
  return execSync("git log --oneline -1 --format=%s", { cwd: dir }).toString().trim();
}
