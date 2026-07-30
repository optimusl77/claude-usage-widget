# Claude Usage Widget

A small, draggable desktop widget for Windows that shows your Claude.ai (Pro/Max) usage: how full your current session and weekly limits are, and when they reset.

![platform](https://img.shields.io/badge/platform-Windows-blue) ![license](https://img.shields.io/badge/license-MIT-lightgrey)

## Download

Grab the latest installer from the [Releases page](../../releases) (`Claude Usage Widget_<version>_x64-setup.exe`) and run it, no admin rights needed.

## How it works

Anthropic doesn't offer a public API for Claude.ai subscription limits, so this app reuses the same response headers Claude Code itself relies on to show usage (`anthropic-ratelimit-unified-*`). This is unofficial and could break if Anthropic changes it. Only your own account is read, nothing is written or shared.

Login works through the official **Claude Code CLI**, not a custom OAuth flow:

1. Click the login button in the widget. It runs `claude login`, which opens your browser for the real Anthropic login.
2. Claude Code writes `~/.claude/.credentials.json`; the widget only reads it.
3. A minimal authenticated request reads the usage headers from the response (this costs a tiny sliver of your quota per refresh).
4. If the session expires, reopen Claude Code briefly to refresh it, or log in again from the widget.

Requires the [Claude Code CLI](https://docs.claude.com/claude-code) to be installed and on your `PATH`.

## Customizing

Right-click the tray icon and open **Settings**: theme (system/light/dark), accent color, compact layout, opacity, always-on-top, autostart, refresh interval. Drag the widget anywhere; its position is remembered.

## Development

Requires [Rust](https://rustup.rs) and Node.js. On Linux, also install the Tauri system libraries (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libsoup-3.0-dev`, `pkg-config`).

```bash
npm install
npm run tauri dev       # live development
cargo test --workspace  # unit tests (auth/usage parsing, settings/cache)
```

Build a Windows installer locally:

```bash
npm run tauri build
```

Alternatively, push a `v*.*.*` tag. `.github/workflows/build-windows.yml` then builds it on GitHub's Windows runners and attaches the installer to a draft release automatically.

## Project layout

- `crates/usage-core`: plain Rust logic for credentials, rate-limit header parsing, and settings/cache. Fully unit-tested, no GUI dependencies.
- `src-tauri`: the Tauri app, including commands, tray menu, window setup, and background polling.
- `ui`: the frontend (plain HTML/CSS/JS, no bundler).

## License

MIT
