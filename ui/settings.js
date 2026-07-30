const { invoke } = window.__TAURI__.core;

const ACCENT_CHOICES = ["#d97757", "#2a78d6", "#1baf7a", "#e87ba4", "#4a3aa7"];
const DEFAULT_THRESHOLDS = { warning: 0.6, serious: 0.8, critical: 0.95 };

let settings = null;

function renderSegmented(container, value) {
  container.querySelectorAll("button").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.value === String(value));
  });
}

function renderSwatches(container, value) {
  container.innerHTML = "";
  for (const hex of ACCENT_CHOICES) {
    const el = document.createElement("div");
    el.className = "swatch" + (hex === value ? " active" : "");
    el.style.background = hex;
    el.dataset.value = hex;
    el.addEventListener("click", () => {
      settings.accentColor = hex;
      renderSwatches(container, hex);
      persist();
    });
    container.appendChild(el);
  }
}

function renderBarColorSwatches(container, value) {
  container.innerHTML = "";

  const autoSwatch = document.createElement("div");
  autoSwatch.className = "swatch swatch-auto" + (value ? "" : " active");
  autoSwatch.title = "Automatic (color reflects usage severity). Right-click to edit thresholds.";
  autoSwatch.addEventListener("click", () => {
    settings.barColor = null;
    renderBarColorSwatches(container, null);
    persist();
  });
  autoSwatch.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    setThresholdsPanelVisible(true);
  });
  container.appendChild(autoSwatch);

  for (const hex of ACCENT_CHOICES) {
    const el = document.createElement("div");
    el.className = "swatch" + (hex === value ? " active" : "");
    el.style.background = hex;
    el.title = hex;
    el.addEventListener("click", () => {
      settings.barColor = hex;
      renderBarColorSwatches(container, hex);
      persist();
    });
    container.appendChild(el);
  }
}

function setThresholdsPanelVisible(visible) {
  document.getElementById("thresholds-panel").hidden = !visible;
  if (visible) document.getElementById("thresholds-panel").scrollIntoView({ block: "nearest" });
}

function renderThresholdInputs() {
  const t = settings.severityThresholds || DEFAULT_THRESHOLDS;
  document.getElementById("threshold-warning").value = Math.round(t.warning * 100);
  document.getElementById("threshold-serious").value = Math.round(t.serious * 100);
  document.getElementById("threshold-critical").value = Math.round(t.critical * 100);
}

function bindThresholdInput(id, key) {
  const input = document.getElementById(id);
  input.addEventListener("change", () => {
    const pct = Number(input.value);
    if (Number.isNaN(pct)) return;
    settings.severityThresholds = {
      ...(settings.severityThresholds || DEFAULT_THRESHOLDS),
      [key]: Math.min(100, Math.max(0, pct)) / 100,
    };
    persist();
  });
}

async function persist() {
  dbg(`settings persist: ${JSON.stringify(settings)}`);
  try {
    await invoke("save_settings", { settings });
  } catch (err) {
    dbg(`settings persist FAILED: ${err}`);
  }
  applyTheme(settings);
}

function bindSegmented(id, key, parse) {
  const container = document.getElementById(id);
  container.addEventListener("click", (e) => {
    const btn = e.target.closest("button");
    if (!btn) return;
    settings[key] = parse(btn.dataset.value);
    renderSegmented(container, settings[key]);
    persist();
  });
}

function bindToggle(id, key) {
  const input = document.getElementById(id);
  input.addEventListener("change", () => {
    settings[key] = input.checked;
    persist();
  });
}

async function init() {
  dbg("settings window init: starting");
  settings = await invoke("get_settings");
  dbg(`settings window init: loaded ${JSON.stringify(settings)}`);
  applyTheme(settings);

  renderSegmented(document.getElementById("theme-segmented"), settings.theme);
  renderSegmented(document.getElementById("interval-segmented"), settings.pollIntervalSecs);
  renderSwatches(document.getElementById("accent-swatches"), settings.accentColor);
  renderBarColorSwatches(document.getElementById("bar-color-swatches"), settings.barColor);
  renderThresholdInputs();

  document.getElementById("estimated-time-toggle").checked = settings.showEstimatedTime;
  document.getElementById("always-on-top-toggle").checked = settings.alwaysOnTop;
  document.getElementById("autostart-toggle").checked = settings.autostart;
  document.getElementById("week-reset-toggle").checked = settings.showWeekReset;
  document.getElementById("opacity-slider").value = settings.opacity;
  document.getElementById("widget-scale-slider").value = settings.widgetScale;

  bindSegmented("theme-segmented", "theme", (v) => v);
  bindSegmented("interval-segmented", "pollIntervalSecs", (v) => Number(v));
  bindToggle("estimated-time-toggle", "showEstimatedTime");
  bindToggle("always-on-top-toggle", "alwaysOnTop");
  bindToggle("autostart-toggle", "autostart");
  bindToggle("week-reset-toggle", "showWeekReset");

  document.getElementById("edit-thresholds-btn").addEventListener("click", () => {
    const panel = document.getElementById("thresholds-panel");
    setThresholdsPanelVisible(panel.hidden);
  });
  document.getElementById("reset-thresholds-btn").addEventListener("click", () => {
    settings.severityThresholds = { ...DEFAULT_THRESHOLDS };
    renderThresholdInputs();
    persist();
  });
  bindThresholdInput("threshold-warning", "warning");
  bindThresholdInput("threshold-serious", "serious");
  bindThresholdInput("threshold-critical", "critical");

  document.getElementById("opacity-slider").addEventListener("input", (e) => {
    settings.opacity = Number(e.target.value);
  });
  document.getElementById("opacity-slider").addEventListener("change", persist);

  document.getElementById("widget-scale-slider").addEventListener("input", (e) => {
    settings.widgetScale = Number(e.target.value);
  });
  document.getElementById("widget-scale-slider").addEventListener("change", persist);
}

init();
