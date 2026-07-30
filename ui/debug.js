// Sends log lines to the Rust backend logger (debug.log in the app data
// directory), so frontend and backend events land in the same file. Also
// catches unhandled errors/promise rejections that would otherwise vanish
// without a trace.
function dbg(msg) {
  try {
    console.log(msg);
  } catch {
    /* ignore */
  }
  try {
    window.__TAURI__.core.invoke("log_frontend", { message: String(msg) });
  } catch {
    /* ignore - Tauri API may not be ready yet */
  }
}

window.addEventListener("error", (e) => {
  dbg(`window.onerror: ${e.message} at ${e.filename}:${e.lineno}:${e.colno}`);
});

window.addEventListener("unhandledrejection", (e) => {
  dbg(`unhandledrejection: ${e.reason}`);
});

dbg(`debug.js loaded on ${location.pathname}`);
