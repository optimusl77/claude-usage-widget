const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const content = document.getElementById("content");
const card = document.getElementById("card");
let countdownTimer = null;
let currentSettings = null;

function applyWidgetTheme(settings) {
  applyTheme(settings);
  card.classList.toggle("compact", settings.compactLayout);
  document.body.style.opacity = String(settings.opacity ?? 1);
}

function statusForUtilization(u) {
  if (u === null || u === undefined) return "good";
  if (u >= 0.95) return "critical";
  if (u >= 0.8) return "serious";
  if (u >= 0.6) return "warning";
  return "good";
}

function formatDuration(seconds) {
  if (seconds <= 0) return "gleich";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  const s = seconds % 60;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

function renderLoading() {
  content.innerHTML = `
    <div class="state-view">
      <div class="spinner"></div>
      <div class="state-desc">Lade Nutzungsdaten…</div>
    </div>`;
}

function renderWaitingForLogin() {
  content.innerHTML = `
    <div class="state-view">
      <div class="spinner"></div>
      <div class="state-title">Warte auf Login im Browser</div>
      <div class="state-desc">Schließe den Login im geöffneten Browserfenster ab. Das kann einen Moment dauern.</div>
      <button class="primary-btn" id="check-again-btn">Ich bin fertig</button>
    </div>`;
  document.getElementById("check-again-btn").addEventListener("click", pollUntilLoggedIn);
}

/// Pollt get_usage_status im Hintergrund, bis die Session nicht mehr
/// "notLoggedIn" ist. Der Rust-Befehl start_claude_login wartet bewusst nicht
/// auf das Prozessende (siehe lib.rs) - dieses Polling ist der eigentliche
/// Fortschrittsindikator fuer den Login.
let loginPollToken = 0;
async function pollUntilLoggedIn() {
  const myToken = ++loginPollToken;
  dbg(`pollUntilLoggedIn: starting (token ${myToken})`);
  for (let i = 0; i < 60; i++) {
    if (myToken !== loginPollToken) {
      dbg(`pollUntilLoggedIn: token ${myToken} superseded, stopping`);
      return;
    }
    try {
      const status = await invoke("get_usage_status");
      dbg(`pollUntilLoggedIn: attempt ${i + 1}/60, status.kind=${status.kind}`);
      if (status.kind !== "notLoggedIn") {
        dbg("pollUntilLoggedIn: login detected, applying status");
        applyStatus(status);
        return;
      }
    } catch (err) {
      dbg(`pollUntilLoggedIn: attempt ${i + 1}/60 threw: ${err}`);
    }
    await new Promise((r) => setTimeout(r, 3000));
  }
  dbg("pollUntilLoggedIn: gave up after 60 attempts (~3 minutes)");
  renderNotLoggedIn();
}

function renderNotLoggedIn() {
  content.innerHTML = `
    <div class="state-view">
      <div class="state-title">Nicht angemeldet</div>
      <div class="state-desc">Melde dich mit deinem Claude-Konto an, um deine Nutzung zu sehen.</div>
      <button class="primary-btn" id="login-btn">Mit Claude anmelden</button>
    </div>`;
  document.getElementById("login-btn").addEventListener("click", async (e) => {
    dbg("login button clicked (state: notLoggedIn)");
    e.target.disabled = true;
    try {
      await invoke("start_claude_login");
      dbg("start_claude_login invoke resolved without error");
      renderWaitingForLogin();
      pollUntilLoggedIn();
    } catch (err) {
      dbg(`start_claude_login invoke FAILED: ${err}`);
      renderError(String(err), null);
    }
  });
}

function renderSessionExpired() {
  content.innerHTML = `
    <div class="state-view">
      <div class="state-title">Sitzung abgelaufen</div>
      <div class="state-desc">Öffne kurz Claude Code, damit sich die Sitzung erneuert, oder melde dich neu an.</div>
      <button class="primary-btn" id="login-btn">Erneut anmelden</button>
    </div>`;
  document.getElementById("login-btn").addEventListener("click", async (e) => {
    dbg("login button clicked (state: sessionExpired)");
    e.target.disabled = true;
    try {
      await invoke("start_claude_login");
      dbg("start_claude_login invoke resolved without error");
      renderWaitingForLogin();
      pollUntilLoggedIn();
    } catch (err) {
      dbg(`start_claude_login invoke FAILED: ${err}`);
      renderError(String(err), null);
    }
  });
}

function renderError(message, staleSnapshot) {
  if (staleSnapshot) {
    renderSnapshot(staleSnapshot, null, true);
    return;
  }
  content.innerHTML = `
    <div class="state-view">
      <div class="state-title">Konnte nicht aktualisieren</div>
      <div class="state-desc">${escapeHtml(message)}</div>
    </div>`;
}

function escapeHtml(s) {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function meterRow(label, window_) {
  const util = window_?.utilization ?? 0;
  const pct = Math.round(util * 1000) / 10;
  const status = statusForUtilization(util);
  return `
    <div class="meter-row">
      <div class="meter-head">
        <span class="meter-label">${label}</span>
        <span class="meter-value">${pct}%</span>
      </div>
      <div class="meter-track">
        <div class="meter-fill" data-status="${status}" style="width:${Math.min(100, Math.max(0, pct))}%"></div>
      </div>
    </div>`;
}

function renderSnapshot(snapshot, subscriptionType, stale) {
  if (countdownTimer) clearInterval(countdownTimer);

  content.innerHTML = `
    <div class="meters">
      ${meterRow("Session (5h)", snapshot.fiveHour)}
      ${meterRow("Woche (7 Tage)", snapshot.sevenDay)}
    </div>
    <div class="reset-line" id="reset-line"></div>
  `;

  const bindingWindow =
    snapshot.representativeClaim === "seven_day" ? snapshot.sevenDay : snapshot.fiveHour;
  const resetLine = document.getElementById("reset-line");

  function tick() {
    if (!bindingWindow?.resetUnix) {
      resetLine.textContent = stale ? "Letzte bekannte Werte (Aktualisierung fehlgeschlagen)" : "";
      return;
    }
    const now = Math.floor(Date.now() / 1000);
    const remaining = bindingWindow.resetUnix - now;
    const prefix = stale ? "Letzte bekannte Werte · " : "";
    resetLine.textContent = `${prefix}Reset in ${formatDuration(Math.max(0, remaining))}`;
  }

  tick();
  countdownTimer = setInterval(tick, 1000);
}

function applyStatus(status) {
  switch (status.kind) {
    case "notLoggedIn":
      renderNotLoggedIn();
      break;
    case "sessionExpired":
      renderSessionExpired();
      break;
    case "ok":
      renderSnapshot(status.snapshot, status.subscriptionType, false);
      break;
    case "error":
      renderError(status.message, status.staleSnapshot);
      break;
    default:
      renderLoading();
  }
}

async function refresh() {
  loginPollToken++; // ein manueller/periodischer Refresh bricht ein laufendes Login-Polling ab
  dbg("refresh: calling get_usage_status");
  try {
    const status = await invoke("get_usage_status");
    dbg(`refresh: got status.kind=${status.kind}`);
    applyStatus(status);
  } catch (err) {
    dbg(`refresh: get_usage_status FAILED: ${err}`);
    renderError(String(err), null);
  }
}

async function init() {
  dbg("widget init: starting");
  renderLoading();
  try {
    currentSettings = await invoke("get_settings");
    dbg(`widget init: settings loaded: ${JSON.stringify(currentSettings)}`);
    applyWidgetTheme(currentSettings);
  } catch (err) {
    dbg(`widget init: get_settings failed, falling back to CSS defaults: ${err}`);
  }

  document.getElementById("refresh-btn").addEventListener("click", refresh);
  document.getElementById("settings-btn").addEventListener("click", () => {
    dbg("settings button clicked");
    invoke("show_settings_window");
  });

  const titlebar = document.querySelector(".titlebar");
  titlebar.addEventListener("mousedown", (e) => {
    if (e.target.closest(".icon-btn")) return;
    getCurrentWindow().startDragging();
  });

  await refresh();

  await listen("usage:poll-tick", refresh);
  await listen("settings:changed", async () => {
    currentSettings = await invoke("get_settings");
    applyWidgetTheme(currentSettings);
  });
}

init();
