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
  src/settings.rs     the settings file
  windows/            extra steps for the installer
ui/                   the dashboard: plain HTML, CSS and canvas, no build step, no dependencies
  themes.css          the themes, each a set of CSS variables in a dark and a light version
  core.js             widget registry, charts, formatting
  widgets/*.js        one file per widget
  settings.js         the settings panel
  app.js              layout and the update loop
tools/preview/        runs the dashboard in a browser on made-up data
```

The backend pushes a sample to the page every tick over Tauri's IPC, with no local web server
and no open port. While the window is hidden it keeps the graphs' history but sends and draws
nothing. Nearly all of Oriel's memory is the WebView2 page; the Rust backend that reads the
sensors is about 11 MB.

The page can only ask Windows to open a fixed list of things (Task Manager, Resource Monitor
and `ms-settings:` pages), end processes the user picks, play, pause or skip the current track,
and set volumes. See `run_action`, `end_processes`, `media_control` and `set_volume` in `main.rs`.

## Writing a widget

A widget is a script in `ui/widgets/` that calls `Oriel.register(id, {name, create(ctx)})` and
returns a root element plus an `update(sample)` function. `ui/widgets/network.js` is a short
one to start from. Add a `<script>` tag for it in `ui/index.html`, then add it to the
dashboard in Settings → Layout.

Use the theme's CSS variables (`var(--ink)`, `var(--cpu)` and so on, see `ui/themes.css`)
rather than fixed colors, so the widget works in every theme, dark and light.

## Working on the dashboard

You don't need the Rust side to work on the dashboard. This opens it in your browser on
made-up data from `tools/preview/demo.js`; edit anything in `ui/` and reload:

```powershell
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1
```

Options go after the page's address, for example `?palette=nord&theme=light`,
`?procs=tree` or `?layout=cpu,processes`. The top of `demo.js` lists them all.

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
