import { describe, it, expect } from "vitest";
import { parseDiff } from "./diffParser";

describe("parseDiff", () => {
  it("parses simple add/delete/context lines", () => {
    const raw = `diff --git a/src/app.ts b/src/app.ts
index abc123..def456 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,4 +1,5 @@
 import React from 'react';
-import { old } from './old';
+import { newThing } from './new';
+import { extra } from './extra';

 function App() {`;

    const result = parseDiff(raw);
    expect(result.files).toHaveLength(1);
    expect(result.files[0].hunks).toHaveLength(1);

    const lines = result.files[0].hunks[0].lines;
    expect(lines[0].type).toBe("context");
    expect(lines[0].content).toBe("import React from 'react';");
    expect(lines[0].oldLineNumber).toBe(1);
    expect(lines[0].newLineNumber).toBe(1);

    expect(lines[1].type).toBe("delete");
    expect(lines[1].content).toBe("import { old } from './old';");
    expect(lines[1].oldLineNumber).toBe(2);
    expect(lines[1].newLineNumber).toBeNull();

    expect(lines[2].type).toBe("add");
    expect(lines[2].content).toBe("import { newThing } from './new';");
    expect(lines[2].oldLineNumber).toBeNull();
    expect(lines[2].newLineNumber).toBe(2);

    expect(lines[3].type).toBe("add");
    expect(lines[3].newLineNumber).toBe(3);
  });

  it("handles multiple files in one diff", () => {
    const raw = `diff --git a/file1.ts b/file1.ts
--- a/file1.ts
+++ b/file1.ts
@@ -1,2 +1,2 @@
-old line
+new line
diff --git a/file2.ts b/file2.ts
--- a/file2.ts
+++ b/file2.ts
@@ -1,3 +1,3 @@
 context
-removed
+added`;

    const result = parseDiff(raw);
    expect(result.files).toHaveLength(2);
    expect(result.files[0].newPath).toBe("file1.ts");
    expect(result.files[1].newPath).toBe("file2.ts");
    expect(result.stats.filesChanged).toBe(2);
  });

  it("detects new file status", () => {
    const raw = `diff --git a/new.ts b/new.ts
new file mode 100644
--- /dev/null
+++ b/new.ts
@@ -0,0 +1,3 @@
+line 1
+line 2
+line 3`;

    const result = parseDiff(raw);
    expect(result.files[0].status).toBe("added");
    expect(result.files[0].additions).toBe(3);
    expect(result.files[0].deletions).toBe(0);
  });

  it("detects deleted file status", () => {
    const raw = `diff --git a/old.ts b/old.ts
deleted file mode 100644
--- a/old.ts
+++ /dev/null
@@ -1,2 +0,0 @@
-line 1
-line 2`;

    const result = parseDiff(raw);
    expect(result.files[0].status).toBe("deleted");
    expect(result.files[0].deletions).toBe(2);
    expect(result.files[0].additions).toBe(0);
  });

  it("computes stats correctly", () => {
    const raw = `diff --git a/a.ts b/a.ts
--- a/a.ts
+++ b/a.ts
@@ -1,5 +1,4 @@
 ctx
-del1
-del2
+add1
 ctx
diff --git a/b.ts b/b.ts
--- a/b.ts
+++ b/b.ts
@@ -1,1 +1,3 @@
 ctx
+add2
+add3`;

    const result = parseDiff(raw);
    expect(result.stats.totalAdditions).toBe(3);
    expect(result.stats.totalDeletions).toBe(2);
    expect(result.stats.filesChanged).toBe(2);
  });

  it("handles empty diff", () => {
    const result = parseDiff("");
    expect(result.files).toHaveLength(0);
    expect(result.stats.totalAdditions).toBe(0);
    expect(result.stats.totalDeletions).toBe(0);
    expect(result.stats.filesChanged).toBe(0);
  });

  it("handles hunk with context function name", () => {
    const raw = `diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,3 +10,4 @@ fn main() {
     let x = 1;
+    let y = 2;
     println!("hi");`;

    const result = parseDiff(raw);
    expect(result.files[0].hunks[0].header).toBe("fn main() {");
  });

  it("detects renamed file", () => {
    const raw = `diff --git a/old_name.ts b/new_name.ts
rename from old_name.ts
rename to new_name.ts
--- a/old_name.ts
+++ b/new_name.ts
@@ -1,1 +1,1 @@
-old
+new`;

    const result = parseDiff(raw);
    expect(result.files[0].status).toBe("renamed");
    expect(result.files[0].oldPath).toBe("old_name.ts");
    expect(result.files[0].newPath).toBe("new_name.ts");
  });

  it("handles multiple hunks in one file", () => {
    const raw = `diff --git a/big.ts b/big.ts
--- a/big.ts
+++ b/big.ts
@@ -1,3 +1,3 @@
 line1
-old2
+new2
 line3
@@ -10,3 +10,4 @@
 line10
+inserted
 line11
 line12`;

    const result = parseDiff(raw);
    expect(result.files[0].hunks).toHaveLength(2);
    expect(result.files[0].hunks[0].oldStart).toBe(1);
    expect(result.files[0].hunks[1].oldStart).toBe(10);
  });
});
