function applyTheme(settings) {
  const root = document.documentElement;
  if (settings.theme === "dark") root.setAttribute("data-theme", "dark");
  else if (settings.theme === "light") root.setAttribute("data-theme", "light");
  else root.removeAttribute("data-theme");
  root.style.setProperty("--accent", settings.accentColor);
}
