# Contributing to Oriel

Thanks for taking a look. Bug reports, ideas and pull requests are all welcome. Oriel's main
promise is that it stays light, so changes that make it heavier (a bundler, a framework, a
background service, polling something every frame) need a very good reason.

## Building from source

You need:

- [Rust](https://rustup.rs)
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with
  the **Desktop development with C++** workload
- The WebView2 runtime, which is built into Windows 11 and comes with Edge on Windows 10

Then:

```powershell
cd src-tauri
cargo run
```

The first build takes a few minutes. The app ends up in `src-tauri\target\debug\oriel.exe`,
and **Start at sign-in** in Settings works with that copy too.

> [!NOTE]
> With Smart App Control on, `cargo build --release` fails with *os error 4551*: Windows won't
> run one of the helper programs Cargo compiles during the build. The `cargo run` debug build
> above works. Please don't turn Smart App Control off to get around this. On most versions of
> Windows it can only be turned back on by reinstalling. Release builds are made by GitHub
> Actions instead (see [Making a release](#making-a-release)).

## How it works

```
src-tauri/            the app: Rust, on Tauri 2 and the system WebView2
  src/main.rs         window placement, tray, settings and actions the page can call
  src/collect.rs      builds one snapshot per tick from the modules below
  src/pdh.rs          Windows performance counters (CPU, GPU engines, disk, network, thermal zone)
  src/gpu.rs          GPU names via DXGI, NVIDIA extras via NVML on its own thread
  src/procs.rs        every process's CPU time, memory and parent in one NtQuerySystemInformation call
  src/lhm.rs          optional LibreHardwareMonitor reader on its own thread
  src/media.rs        what's playing, from Windows' media controls, on its own thread
  src/audio.rs        the volume mixer: output and per-app volume via Core Audio, on its own thread
  src/plugins.rs      serves widgets from the plugin folder over the oriel-plugins:// scheme
  src/settings.rs     the settings file
  windows/            extra steps for the installer
ui/                   the dashboard: plain HTML, CSS and canvas, no build step, no dependencies
  themes.css          the themes, each a set of CSS variables in a dark and a light version
  core.js             widget registry, charts, formatting
  widgets/*.js        one file per widget
  plugins.js          loads widgets from the plugin folder and keeps a broken one contained
  settings.js         the settings panel
  app.js              layout and the update loop
docs/examples/        an example plugin to copy
tools/preview/        runs the dashboard in a browser on made-up data
```

The backend pushes a sample to the page every tick over Tauri's IPC, with no local web server
and no open port. While the window is hidden it keeps the graphs' history but sends and draws
nothing. Nearly all of Oriel's memory is the WebView2 page; the Rust backend that reads the
sensors is about 11 MB.

The page can only ask Windows to open a fixed list of things (Task Manager, Resource Monitor
and `ms-settings:` pages), end processes the user picks, play, pause or skip the current track,
and set volumes. See `run_action`, `end_processes`, `media_control` and `set_volume` in `main.rs`.

With **Load widgets from the plugins folder** on, the page also loads `.js` and `.css` files
from `%APPDATA%\Oriel\widgets` through a custom URI scheme (`http://oriel-plugins.localhost/`
on Windows), which the CSP allows for scripts and styles and nothing else. `plugins.rs` serves
only plain files directly inside that folder, up to 512 KB each, and nothing at all while the
setting is off.

## Writing a widget

A widget is a script in `ui/widgets/` that calls `Oriel.register(id, {name, create(ctx)})` and
returns a root element plus an `update(sample)` function. `ui/widgets/network.js` is a short
one to start from. Add a `<script>` tag for it in `ui/index.html`, then add it to the
dashboard in Settings → Layout.

Use the theme's CSS variables (`var(--ink)`, `var(--cpu)` and so on, see `ui/themes.css`)
rather than fixed colors, so the widget works in every theme, dark and light.

### Writing a plugin

A plugin is a widget that lives in a folder instead of in Oriel's source, so nobody has to
rebuild anything. Put `name.js` (and, if you want, `name.css` beside it) in
`%APPDATA%\Oriel\widgets`, turn on **Settings → Plugins → Load widgets from the plugins folder**
(the **Open plugins folder** button there creates and opens it) and restart Oriel. The widget
then shows in Settings → Layout, marked *(plugin)*. [`docs/examples/world-clock.js`](docs/examples/world-clock.js)
and its `.css` are a complete small one to copy.

> [!WARNING]
> A plugin runs inside the dashboard with the same rights as the built-in widgets: it can read
> every sample, and call everything the dashboard can (end processes, change volume, open
> Windows settings pages). Only use plugins from people you trust, as with browser extensions.
> The page's content security policy stops a plugin loading anything from the internet (`fetch`,
> `WebSocket`, remote scripts, images and fonts) but is a safety net, not a sandbox.

**The API** is the one the built-in widgets use, and `ui/core.js` documents it at the top:

```js
Oriel.register("my-widget", {      // id: lowercase letters, digits and dashes, up to 32
  name: "My widget",               // shown in Settings → Layout
  description: "What it shows",
  color: "s1",                     // a theme color for its dot there
  size: { w: 1, h: 1 },            // width within its row, and how tall the row should be
  create(ctx) {                    // once per tile; ctx: {info, inst, chart, go}
    const root = Oriel.el(`<article class="tile"><header><h2>My widget</h2></header>…</article>`);
    return {
      root,                        // the tile's element
      update(sample) { … },        // about once a second while the page is on screen
    };
  },
});
```

Helpers on `Oriel`: `el(html)` and `refs(root)` to build a tile and find its `data-k` parts,
`setText` and `setHTML` (they skip the DOM when nothing changed), `esc` for anything not yours
that goes into HTML, `fmt` (bytes, rates, durations), `ctx.chart(element, {series, max})` for a
graph, and `ctx.go(action, title)` for an "open in Windows" button.

**The sample** passed to `update` has these fields (see `src-tauri/src/collect.rs`):

| Field | |
|---|---|
| `t`, `uptime` | time of the sample and seconds since boot |
| `cpu` | `total` (%), `cores[]`, `mhz`, `temp`, `power`, `fans[]` (some can be null) |
| `mem` | `used`, `total`, `pct`, `commit_used`, `commit_limit` |
| `gpus[]` | per graphics card: `id`, `util`, `vram_used`, `temp`, `power`, … (names are in `ctx.info.gpus`) |
| `net`, `disk` | `net.down`/`up` and `disk.read`/`write`/`active`, bytes per second |
| `battery` | null on a desktop |
| `procs[]`, `drives[]` | processes (`pid`, `ppid`, `name`, `cpu`, `mem`) and drives |
| `media`, `audio` | what's playing and the volume mixer, when those widgets are on a page |

`Oriel.state.info` has the host, OS and hardware, `Oriel.state.settings` the settings.

**Looks.** Use the theme variables (`var(--ink)`, `var(--muted)`, `var(--surface)`,
`var(--s1)`, `var(--warn)`, `var(--crit)`, …) so the widget follows the theme, and the
classes the built-ins use: `tile` (with `<header><h2>`), `big` and `mid` for numbers,
`label`, `facts`, `chip warn|crit`, `chart`. Start every selector in your `.css` with your
own tile's class, so it can't change anything else.

**Mistakes are contained.** An error when the file loads, in `create` or in `update` turns that
tile into an error tile with the file name and message, and everything else keeps running.
Settings → Plugins lists the problems. If a plugin uses an id that's already taken, by a
built-in widget or an earlier file (files load in alphabetical order), the first one wins and
the clash is reported.

**Trying it without Oriel.** The preview (below) can load a plugin: serve your folder with
anything that adds `Access-Control-Allow-Origin: *`, then add
`?plugin=http://127.0.0.1:5300/my-widget.js&plugin_css=http://127.0.0.1:5300/my-widget.css&layout=cpu,my-widget`
to the page's address. `?plugins=on` loads a set of deliberately broken made-up plugins.

## Working on the dashboard

You don't need the Rust side to work on the dashboard. This opens it in your browser on
made-up data from `tools/preview/demo.js`; edit anything in `ui/` and reload:

```powershell
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1
```

Options go after the page's address, for example `?palette=nord&theme=light`,
`?procs=tree`, `?layout=cpu,processes` or `?pages=Resources:cpu,gpu|processes;Media:media`.
The top of `demo.js` lists them all.

Add `-Screenshots` to render the images in the README again (it uses Edge or Chrome,
headless). The demo data is seeded, so the pictures only change when the dashboard does.

## Making a release

Releases are built by GitHub Actions (`.github/workflows/release.yml`), not on a PC, so Smart
App Control and local toolchains don't matter:

1. Set `version` in `src-tauri/Cargo.toml` (the installer takes its version from there).
2. Commit, then tag and push: `git tag v0.2.0` and `git push origin v0.2.0`.
3. The workflow builds `Oriel-0.2.0-setup.exe`, `Oriel-0.2.0-portable.zip` and
   `SHA256SUMS.txt`, and drafts a release with them. Add notes and publish it.

The installer is Tauri's NSIS installer. It installs per user, needs no admin rights and, when
"Delete the application data" is ticked on uninstall, also removes the settings and the sign-in
entry (`src-tauri/windows/installer-hooks.nsh`).

Releases aren't code-signed yet, so SmartScreen may warn about them and Smart App Control may
block them. Signing would slot into the workflow's Build step once there's a certificate.
