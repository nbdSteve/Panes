import { describe, it, expect, vi, afterEach } from "vitest";
import { copyTextToClipboard } from "./clipboard";

// jsdom locks navigator.clipboard behind a prototype getter. Tests
// install a writable override per-case and restore the default afterwards.
const CLIPBOARD_DESCRIPTOR = Object.getOwnPropertyDescriptor(
  Navigator.prototype,
  "clipboard",
);

function setClipboard(impl: { writeText?: (s: string) => Promise<void> } | undefined) {
  if (impl === undefined) {
    // @ts-expect-error — intentionally shadow the prototype getter.
    delete (navigator as Navigator).clipboard;
  } else {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: impl,
    });
  }
}

function setExecCommand(fn: ((cmd: string) => boolean) | undefined) {
  if (fn === undefined) {
    // @ts-expect-error — cleanup test-only stub
    delete (document as Document).execCommand;
  } else {
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: fn,
    });
  }
}

afterEach(() => {
  // @ts-expect-error — reset overrides between tests
  delete (navigator as Navigator).clipboard;
  if (CLIPBOARD_DESCRIPTOR) {
    Object.defineProperty(Navigator.prototype, "clipboard", CLIPBOARD_DESCRIPTOR);
  }
  // @ts-expect-error — remove stub execCommand if one was installed
  delete (document as Document).execCommand;
});

describe("copyTextToClipboard", () => {
  it("uses navigator.clipboard.writeText when available", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText });

    const ok = await copyTextToClipboard("hello");

    expect(writeText).toHaveBeenCalledWith("hello");
    expect(ok).toBe(true);
  });

  it("falls back to execCommand when writeText throws", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    setClipboard({ writeText });
    const execCommand = vi.fn().mockReturnValue(true);
    setExecCommand(execCommand);

    const ok = await copyTextToClipboard("fallback body");

    expect(writeText).toHaveBeenCalledWith("fallback body");
    expect(execCommand).toHaveBeenCalledWith("copy");
    expect(ok).toBe(true);
  });

  it("returns false when both paths fail", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    setClipboard({ writeText });
    setExecCommand(() => false);

    const ok = await copyTextToClipboard("nope");

    expect(ok).toBe(false);
  });

  it("returns false when neither path is available", async () => {
    // No clipboard, no execCommand — the helper must not throw.
    setClipboard(undefined);
    setExecCommand(undefined);

    const ok = await copyTextToClipboard("nothing works");
    expect(ok).toBe(false);
  });
});
