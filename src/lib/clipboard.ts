/**
 * Copy text to the clipboard, with a fallback for environments that don't
 * expose `navigator.clipboard.writeText` — notably the Tauri webview under
 * older macOS versions and any non-secure context used during testing.
 *
 * Returns true on success so callers can drive a transient "copied" UI
 * without needing to introspect a throw.
 */
export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the legacy path — some environments reject
      // writeText when the document isn't focused.
    }
  }
  try {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.top = "-1000px";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(textarea);
    return ok;
  } catch {
    return false;
  }
}
