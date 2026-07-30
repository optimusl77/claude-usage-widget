// Sendet Log-Zeilen an den Rust-Backend-Logger (debug.log im App-Datenverzeichnis),
// damit Frontend- und Backend-Ereignisse in derselben Datei landen. Fängt außerdem
// unbehandelte Fehler/Promise-Rejections ab, die sonst spurlos verschwinden würden.
function dbg(msg) {
  try {
    console.log(msg);
  } catch {
    /* ignore */
  }
  try {
    window.__TAURI__.core.invoke("log_frontend", { message: String(msg) });
  } catch {
    /* ignore - Tauri API evtl. noch nicht bereit */
  }
}

window.addEventListener("error", (e) => {
  dbg(`window.onerror: ${e.message} at ${e.filename}:${e.lineno}:${e.colno}`);
});

window.addEventListener("unhandledrejection", (e) => {
  dbg(`unhandledrejection: ${e.reason}`);
});

dbg(`debug.js loaded on ${location.pathname}`);
