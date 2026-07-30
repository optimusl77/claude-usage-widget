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

async function persist() {
  await invoke("save_settings", { settings });
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
  settings = await invoke("get_settings");
  applyTheme(settings);

  renderSegmented(document.getElementById("theme-segmented"), settings.theme);
  renderSegmented(document.getElementById("interval-segmented"), settings.pollIntervalSecs);
  renderSwatches(document.getElementById("accent-swatches"), settings.accentColor);

  document.getElementById("compact-toggle").checked = settings.compactLayout;
  document.getElementById("always-on-top-toggle").checked = settings.alwaysOnTop;
  document.getElementById("autostart-toggle").checked = settings.autostart;
  document.getElementById("opacity-slider").value = settings.opacity;

  bindSegmented("theme-segmented", "theme", (v) => v);
  bindSegmented("interval-segmented", "pollIntervalSecs", (v) => Number(v));
  bindToggle("compact-toggle", "compactLayout");
  bindToggle("always-on-top-toggle", "alwaysOnTop");
  bindToggle("autostart-toggle", "autostart");

  document.getElementById("opacity-slider").addEventListener("input", (e) => {
    settings.opacity = Number(e.target.value);
  });
  document.getElementById("opacity-slider").addEventListener("change", persist);
}

init();
