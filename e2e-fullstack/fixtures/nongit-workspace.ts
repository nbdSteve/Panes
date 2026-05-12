import { existsSync, mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

/**
 * Create a plain (non-git) workspace directory. Used to exercise the
 * shadow version tracker path — the frontend's Inspect/Revert flow
 * should work identically whether or not a .git directory exists.
 */
export function createNonGitWorkspace(): string {
  const dir = mkdtempSync(resolve(tmpdir(), "panes-nongit-ws-"));
  writeFileSync(resolve(dir, "README.md"), "# Plain workspace\n");
  return dir;
}

export function fileExists(dir: string, rel: string): boolean {
  return existsSync(resolve(dir, rel));
}
