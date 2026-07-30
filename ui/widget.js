const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const content = document.getElementById("content");
const card = document.getElementById("card");
let countdownTimer = null;
let currentSettings = null;
let lastStatus = null;

function applyWidgetTheme(settings) {
  applyTheme(settings);
  // Scales the whole widget (spacing, text, bars) by changing the root
  // font-size that all widget CSS is defined relative to (rem units), then
  // the actual OS window is resized to match on the Rust side. This is more
  // robust across rendering engines than a CSS zoom/transform hack, which
  // can disagree with the native window size and produce a scrollbar.
  const scale = settings.widgetScale || 1;
  document.documentElement.style.fontSize = `${10 * scale}px`;
  document.body.style.opacity = String(settings.opacity ?? 1);
}

function statusForUtilization(u, thresholds) {
  if (u === null || u === undefined) return "good";
  const t = thresholds || { warning: 0.6, serious: 0.8, critical: 0.95 };
  if (u >= t.critical) return "critical";
  if (u >= t.serious) return "serious";
  if (u >= t.warning) return "warning";
  return "good";
}

function formatDuration(seconds) {
  if (seconds <= 0) return "now";
  // Minute granularity only, never seconds: this feeds a live countdown, and
  // updating a "…m Ns" label every second was distracting flicker for no
  // real benefit at these timescales (5h/7d windows).
  const totalMinutes = Math.ceil(seconds / 60);
  const h = Math.floor(totalMinutes / 60);
  const m = totalMinutes % 60;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function formatRelative(targetUnix) {
  const now = Math.floor(Date.now() / 1000);
  const remaining = targetUnix - now;
  return formatDuration(Math.max(0, remaining));
}

function renderLoading() {
  content.innerHTML = `
    <div class="state-view">
      <div class="spinner"></div>
      <div class="state-desc">Loading usage data…</div>
    </div>`;
}

function renderWaitingForLogin() {
  content.innerHTML = `
    <div class="state-view">
      <div class="spinner"></div>
      <div class="state-title">Waiting for login in your browser</div>
      <div class="state-desc">Complete the login in the browser window that opened. This can take a moment.</div>
      <button class="primary-btn" id="check-again-btn">I'm done</button>
    </div>`;
  document.getElementById("check-again-btn").addEventListener("click", pollUntilLoggedIn);
}

// Polls get_usage_status in the background until the session is no longer
// "notLoggedIn". The start_claude_login Rust command deliberately doesn't
// wait for the process to exit (see lib.rs); this polling is the actual
// login progress indicator.
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
  dbg("pollUntilLoggedIn: gave up after 60 attempts (about 3 minutes)");
  renderNotLoggedIn();
}

function renderNotLoggedIn() {
  content.innerHTML = `
    <div class="state-view">
      <div class="state-title">Not signed in</div>
      <div class="state-desc">Sign in with your Claude account to see your usage.</div>
      <button class="primary-btn" id="login-btn">Sign in with Claude</button>
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
      <div class="state-title">Session expired</div>
      <div class="state-desc">Open Claude Code briefly to refresh the session, or sign in again.</div>
      <button class="primary-btn" id="login-btn">Sign in again</button>
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
    renderSnapshot(staleSnapshot, true);
    return;
  }
  content.innerHTML = `
    <div class="state-view">
      <div class="state-title">Couldn't refresh</div>
      <div class="state-desc">${escapeHtml(message)}</div>
    </div>`;
}

function escapeHtml(s) {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

function meterRow(label, window_, opts) {
  const { barColorOverride, showEstimate, thresholds, showResetInfo } = opts;
  const util = window_?.utilization ?? 0;
  const pct = Math.round(util * 1000) / 10;
  const status = statusForUtilization(util, thresholds);
  const colorStyle = barColorOverride ? ` background-color:${barColorOverride};` : "";

  let resetLine = "";
  let estimateLine = "";
  if (showResetInfo) {
    if (window_?.resetUnix) {
      resetLine = `<div class="meter-sub" data-target-unix="${window_.resetUnix}" data-prefix="Resets in "></div>`;
    }
    if (showEstimate) {
      estimateLine = window_?.estimatedFullUnix
        ? `<div class="meter-estimate" data-target-unix="${window_.estimatedFullUnix}" data-prefix="~Full in "></div>`
        : `<div class="meter-estimate meter-estimate-pending">Gathering usage data…</div>`;
    }
  }

  return `
    <div class="meter-row">
      <div class="meter-head">
        <span class="meter-label">${label}</span>
        <span class="meter-value">${pct}%</span>
      </div>
      <div class="meter-track">
        <div class="meter-fill" data-status="${status}" style="width:${Math.min(100, Math.max(0, pct))}%;${colorStyle}"></div>
      </div>
      ${resetLine}
      ${estimateLine}
    </div>`;
}

function renderSnapshot(snapshot, stale) {
  if (countdownTimer) clearInterval(countdownTimer);

  const barColorOverride = currentSettings?.barColor || null;
  const showEstimate = !!currentSettings?.showEstimatedTime;
  const thresholds = currentSettings?.severityThresholds;
  const showWeekReset = currentSettings?.showWeekReset ?? true;

  content.innerHTML = `
    <div class="meters">
      ${meterRow("Session (5h)", snapshot.fiveHour, { barColorOverride, showEstimate, thresholds, showResetInfo: true })}
      ${meterRow("Week (7 days)", snapshot.sevenDay, { barColorOverride, showEstimate, thresholds, showResetInfo: showWeekReset })}
    </div>
    ${stale ? '<div class="stale-note">Showing last known values, update failed</div>' : ""}
  `;

  function tick() {
    document.querySelectorAll("[data-target-unix]").forEach((el) => {
      const target = Number(el.dataset.targetUnix);
      const prefix = el.dataset.prefix || "";
      el.textContent = `${prefix}${formatRelative(target)}`;
    });
  }
  tick();
  countdownTimer = setInterval(tick, 15000);
}

function applyStatus(status) {
  lastStatus = status;
  switch (status.kind) {
    case "notLoggedIn":
      renderNotLoggedIn();
      break;
    case "sessionExpired":
      renderSessionExpired();
      break;
    case "ok":
      renderSnapshot(status.snapshot, false);
      break;
    case "error":
      renderError(status.message, status.staleSnapshot);
      break;
    default:
      renderLoading();
  }
}

async function refresh() {
  loginPollToken++; // a manual/periodic refresh cancels any running login poll
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
    // Re-render the last known status with the new settings (bar color,
    // estimated time toggle) without triggering another network poll.
    if (lastStatus) applyStatus(lastStatus);
  });
}

init();
