// What OS the app runs on, and the words and keys that differ between them.
// Detected from the webview (WKWebView, WebView2, WebKitGTK); `?os=windows`
// or `?os=linux` overrides it in `bun run dev` for design work.

export type OS = "macos" | "windows" | "linux";

function detect(): OS {
  if (typeof navigator === "undefined") return "macos";
  const forced = typeof location !== "undefined" ? new URLSearchParams(location.search).get("os") : null;
  if (forced === "macos" || forced === "windows" || forced === "linux") return forced;
  const s = `${navigator.platform} ${navigator.userAgent}`;
  if (/Mac/i.test(s)) return "macos";
  if (/Win/i.test(s)) return "windows";
  return "linux";
}

export const os: OS = detect();
export const isMac = os === "macos";

/** "⌘K" on macOS, "Ctrl+K" elsewhere. `keys` is what follows the modifier. */
export function mod(keys: string): string {
  return isMac ? `⌘${keys}` : `Ctrl+${keys}`;
}

/** The Shift key's label. */
export const shift = isMac ? "⇧" : "Shift";

/** Where the session is kept (D9). */
export const secretStore = isMac ? "the macOS Keychain" : os === "windows" ? "Windows Credential Manager" : "the Secret Service (GNOME Keyring or KWallet)";

/** Tauri serves custom protocols as `http://<scheme>.localhost` on Windows. */
export function protocolUrl(scheme: string, path: string): string {
  return os === "windows" ? `http://${scheme}.localhost/${path}` : `${scheme}://localhost/${path}`;
}

// Mark the document so CSS can adapt (window controls, effects).
if (typeof document !== "undefined") document.documentElement.dataset.os = os;
