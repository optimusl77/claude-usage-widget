const { invoke } = window.__TAURI__.core;

const ACCENT_CHOICES = ["#d97757", "#2a78d6", "#1baf7a", "#e87ba4", "#4a3aa7"];

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
  autoSwatch.title = "Automatic (color reflects usage severity)";
  autoSwatch.addEventListener("click", () => {
    settings.barColor = null;
    renderBarColorSwatches(container, null);
    persist();
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

  document.getElementById("estimated-time-toggle").checked = settings.showEstimatedTime;
  document.getElementById("always-on-top-toggle").checked = settings.alwaysOnTop;
  document.getElementById("autostart-toggle").checked = settings.autostart;
  document.getElementById("opacity-slider").value = settings.opacity;
  document.getElementById("widget-scale-slider").value = settings.widgetScale;

  bindSegmented("theme-segmented", "theme", (v) => v);
  bindSegmented("interval-segmented", "pollIntervalSecs", (v) => Number(v));
  bindToggle("estimated-time-toggle", "showEstimatedTime");
  bindToggle("always-on-top-toggle", "alwaysOnTop");
  bindToggle("autostart-toggle", "autostart");

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
