# OpenClaudia UI

Browser-based chat interface for [OpenClaudia](https://github.com/dollspace-gay/OpenClaudia). Auto-finds free ports, launches the OpenClaudia proxy as a child process, and serves a lightweight web GUI on a single port.

## Features

- **Zero configuration** — finds available ports, starts the proxy, opens the UI
- **Light/dark mode** — system preference detection with manual toggle
- **SSE streaming** — responses appear in real time
- **Command palette** — type `/` for slash command autocomplete
- **Lightweight** — 6 static files, no JavaScript framework, ~200 lines of Rust
- **Taskbar-safe** — 60px bottom cushion for Windows taskbar

## Quick Start

### Prerequisites

- [OpenClaudia](https://github.com/dollspace-gay/OpenClaudia) built and on your PATH (or `openclaudia-ui` placed next to `openclaudia`)
- Rust toolchain (for building)

### Build

```bash
git clone https://github.com/riziles/openclaudia-ui.git
cd openclaudia-ui
cargo build --release
```

Copy the binary and static files next to `openclaudia`:

```bash
cp target/release/openclaudia-ui* /path/to/openclaudia/
cp -r static /path/to/openclaudia/
```

### Run

From any project directory with an `.openclaudia/config.yaml`:

```bash
openclaudia-ui
```

Opens the web UI on a random available port. Press `Ctrl+C` to stop.

## How It Works

```
Browser ──→ openclaudia-ui (port A) ──→ openclaudia proxy (port B, internal)
                │                              │
                ├─ Serves static/ files        ├─ /v1/chat/completions
                └─ Proxies /v1/* to port B     └─ OpenAI-compatible API
```

The browser talks to a single port. `openclaudia-ui` proxies all `/v1/*` API requests to an internal OpenClaudia proxy instance, so the web UI JavaScript doesn't need to know what port the proxy is on.

## Dev Mode

For hot-reloading CSS/JS changes (no rebuild needed):

```bash
OPENCLAUDIA_WEB_GUI_DEV=1 openclaudia-ui
```

Serves files directly from the `static/` directory instead of the embedded copies. Edit CSS, refresh the browser — changes appear instantly.

## Structure

```
openclaudia-ui/
├── Cargo.toml
├── src/
│   └── main.rs          # Launcher: port discovery, proxy spawn, HTTP server
└── static/
    ├── index.html        # HTML shell
    ├── style.css         # Light/dark theme, layout
    ├── app.js            # Main application, theme toggle, slash commands
    ├── api.js            # SSE streaming client for the proxy
    ├── chat.js           # Chat renderer with content-visibility: auto
    └── palette.js        # Slash command palette with autocomplete
```
