// Stand-in for the Rust backend, so the dashboard runs in a normal browser with made-up data.
// Loaded before ui/core.js by preview.ps1. Everything is seeded, so every run looks the same.
//
// URL options, all optional:
//   palette=nord  theme=dark|light|auto  settings=1 (open the panel)  time=2026-10-18T14:32
//   layout=cpu,gpu|processes (rows split by |)  procs=group|tree  click=chrome,Code (rows to open or close)
//   pages=Resources:cpu,gpu,memory|network,disk,processes;Media:media,processes (pages split by ;)
//   page=2 (the page on screen)  rotate=10 (seconds between pages)
//   media=off (nothing playing)  audio=off (no output device)
//   windows=none (nothing open)  windows=many (thirty windows)  windows=front (Oriel itself in front)
(() => {
  "use strict";
  const GB = 1024 ** 3, MB = 1024 ** 2;
  const q = new URLSearchParams(location.search);

  // A fixed clock for screenshots.
  if (q.get("time")) {
    const RealDate = Date;
    const offset = new RealDate(q.get("time")).getTime() - RealDate.now();
    if (Number.isFinite(offset)) {
      globalThis.Date = class extends RealDate {
        constructor(...a) { super(...(a.length ? a : [RealDate.now() + offset])); }
        static now() { return RealDate.now() + offset; }
      };
    }
  }

  // mulberry32: small, seedable, good enough for fake sensor noise.
  const prng = seed => () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let x = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
  const rnd = prng(0x4f52494c);
  const noise = () => (rnd() + rnd() + rnd() + rnd() - 2) * 1.2; // roughly normal, sd ≈ 0.7
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  const r1 = v => Math.round(v * 10) / 10;
  const r2 = v => Math.round(v * 100) / 100;

  if (["group", "tree"].includes(q.get("procs"))) {
    try { localStorage.setItem("oriel.processes.view", q.get("procs")); } catch {}
  }
  if (q.get("page")) {
    try { localStorage.setItem("oriel.page", String(Number(q.get("page")) - 1)); } catch {}
  }
  const rows = s => (s || "").split("|").map(row => row.split(",").filter(Boolean)).filter(row => row.length);
  const layout = rows(q.get("layout"));
  const pages = (q.get("pages") || "").split(";").filter(Boolean).map((p, i) => {
    const at = p.indexOf(":");
    return at < 0 ? { name: `Page ${i + 1}`, layout: rows(p) } : { name: p.slice(0, at).trim() || `Page ${i + 1}`, layout: rows(p.slice(at + 1)) };
  }).slice(0, 9);

  let settings = {
    theme: ["dark", "light", "auto"].includes(q.get("theme")) ? q.get("theme") : "dark",
    palette: /^[a-z0-9-]{1,32}$/.test(q.get("palette") || "") ? q.get("palette") : "oriel",
    monitor: "auto", fullscreen: true, interval_ms: 1000, graph_seconds: 300,
    pages: pages.length ? pages : [{ name: "Dashboard", layout: layout.length ? layout : [["cpu", "gpu", "memory"], ["network", "disk", "processes"]] }],
    rotate_seconds: Number(q.get("rotate")) || 0,
    limits: { cpu_temp: [90, 97], gpu_temp: [83, 90], mem: [85, 95], drive: [90, 95] },
    burn_in_shift: true, gpu: false, nvml: true, lhm_url: "http://127.0.0.1:8085/data.json",
  };
  const GPU_ID = "0x00000000_0x0000e1c7";
  const info = {
    host: "DESKTOP-7Q2K4M", os: "Windows 11 24H2", version: "0.1.0",
    cpu: { name: "AMD Ryzen 7 7800X3D 8-Core Processor", cores: 8, threads: 16, base_mhz: 4200 },
    mem_total: 31.9 * GB,
    gpus: [{ id: GPU_ID, name: "NVIDIA GeForce RTX 4070", vendor: "nvidia", integrated: false, vram_total: 12 * GB, shared_total: 15.9 * GB, sensors: true }],
    has_battery: false,
  };

  // What the machine is doing, by tick. The first stretch is scripted so the graphs tell a story
  // (a Rust build, a local model on the GPU, a game update downloading); after that it's random.
  const SCRIPT = {
    build: [[30, 85], [150, 178], [236, 330]],
    gpu: [[0, 20], [95, 205], [262, 400]],
    download: [[55, 140]],
  };
  const phases = { build: false, gpu: false, download: false };
  const left = { build: 0, gpu: 0, download: 0 };
  const odds = { build: [1 / 120, 25, 70], gpu: [1 / 150, 40, 120], download: [1 / 400, 30, 90] };
  function advancePhases(i) {
    for (const k of Object.keys(phases)) {
      if (i < 330) {
        phases[k] = SCRIPT[k].some(([a, b]) => i >= a && i < b);
      } else if (left[k] > 0) {
        left[k]--;
        phases[k] = left[k] > 0;
      } else if (rnd() < odds[k][0]) {
        left[k] = Math.round(odds[k][1] + rnd() * (odds[k][2] - odds[k][1]));
        phases[k] = true;
      }
    }
  }

  // Busier cores first, the way Windows schedules light work.
  const coreWeight = Array.from({ length: 16 }, (_, k) => 2.4 * Math.exp(-k / 3.5) + 0.15);
  const meanWeight = coreWeight.reduce((a, b) => a + b) / 16;
  const st = {
    i: 0, cpu: 6, temp: 46, gpu: 3, gtemp: 38, mem: 17.2, vram: 1.4, loadedUntil: -1,
    down: 1.2e5, up: 4e4, read: 1e6, write: 2e6, readTo: 1e6, writeTo: 2e6, burst: 0, cores: new Array(16).fill(0),
  };

  function step() {
    const i = st.i++;
    advancePhases(i);
    const { build, gpu, download } = phases;

    // CPU: a mean-reverting walk towards "idle" or "compiling", with the odd spike while idle.
    st.cpu += ((build ? 74 : 6) - st.cpu) * 0.3 + noise() * (build ? 7 : 2);
    if (!build && rnd() < 0.03) st.cpu += 6 + rnd() * 10;
    st.cpu = clamp(st.cpu, 0.6, 99);
    st.cores = st.cores.map((c, k) => {
      const target = build ? st.cpu + noise() * 14 : (st.cpu * coreWeight[k]) / meanWeight + noise() * 2;
      return r1(clamp(c + (target - c) * 0.7, 0, 100));
    });
    st.temp += (42 + 0.42 * st.cpu - st.temp) * 0.07 + noise() * 0.4;

    // GPU: a local model answering prompts, which keeps 8 GB loaded for a while after.
    st.gpu += ((gpu ? 93 : 3) - st.gpu) * 0.4 + noise() * (gpu ? 4 : 1.2);
    if (gpu && rnd() < 0.05) st.gpu -= 25 + rnd() * 30;
    st.gpu = clamp(st.gpu, 0, 100);
    st.gtemp += (36 + 0.36 * st.gpu - st.gtemp) * 0.06 + noise() * 0.3;
    if (gpu) st.loadedUntil = i + 120;
    st.vram += ((i < st.loadedUntil ? 9.5 : 1.4) - st.vram) * 0.25;

    const loaded = i < st.loadedUntil;
    st.mem += (17.2 + (build ? 3.4 : 0) + (loaded ? 0.6 : 0) - st.mem) * 0.08 + noise() * 0.04;

    // Network: background chatter and the occasional page load, or a game update.
    const burst = !download && rnd() < 0.04;
    st.down = download ? clamp(46e6 + noise() * 5e6, 2e7, 6e7)
      : burst ? 2e6 + rnd() * 7e6 : clamp(st.down * 0.6 + (1.2e5 + noise() * 8e4) * 0.4, 1e4, 1e6);
    st.up = download ? 7e5 + noise() * 1.5e5 : rnd() < 0.03 ? 6e5 + rnd() * 1.4e6 : clamp(4e4 + noise() * 2.5e4, 3e3, 2e5);

    // Disk: the download lands on disk; a build reads sources and writes artifacts in bursts of a few seconds.
    if (--st.burst <= 0) {
      st.burst = 2 + Math.floor(rnd() * 4);
      st.readTo = build ? (rnd() < 0.4 ? 3e7 + rnd() * 8e7 : 2e6 + rnd() * 8e6) : rnd() < 0.15 ? 3e6 + rnd() * 1.5e7 : 2e5 + rnd() * 1e6;
      st.writeTo = build ? (rnd() < 0.45 ? 4e7 + rnd() * 1e8 : 4e6 + rnd() * 1.5e7) : rnd() < 0.15 ? 2e6 + rnd() * 1e7 : 3e5 + rnd() * 1.5e6;
    }
    st.read += (st.readTo - st.read) * 0.6;
    st.write = download ? st.down * (0.9 + rnd() * 0.2) : st.write + (st.writeTo - st.write) * 0.6;

    const gpuSample = {
      id: GPU_ID, util: r1(st.gpu), vram_used: (st.vram + noise() * 0.02) * GB, shared_used: 0.15 * GB, asleep: false,
      temp: Math.round(st.gtemp), power: 14 + st.gpu * 1.8 + noise() * 3, power_limit: 200,
      clock: st.gpu > 20 ? 2715 + noise() * 25 : 210 + st.gpu * 30, mem_clock: st.gpu > 20 ? 10501 : 405,
      fan: st.gtemp < 50 ? 0 : Math.round(30 + (st.gtemp - 50) * 1.4),
    };
    return {
      t: Date.now() / 1000,
      cpu: {
        total: r1(st.cpu), cores: st.cores, mhz: 3550 + 12.5 * st.cpu + noise() * 60, temp: Math.round(st.temp), temp_source: "lhm",
        power: 24 + 0.75 * st.cpu + noise() * 1.5, fans: [{ name: "CPU", rpm: Math.round(620 + 22 * (st.temp - 40)) }],
      },
      mem: { used: st.mem * GB, total: info.mem_total, pct: (st.mem * GB / info.mem_total) * 100, commit_used: (st.mem + 5.8) * GB, commit_limit: 36.6 * GB },
      gpus: [gpuSample],
      net: { down: st.down, up: st.up },
      disk: { read: st.read, write: st.write, active: r1(clamp(0.5 + (st.read + st.write) / 8e6, 0, 100)) },
      battery: null,
      uptime: 187200 + i,
    };
  }

  // The process tree of a typical Windows 11 desktop. P(name, private MB, idle CPU %, children,
  // instances, share of a build's CPU): instances split the memory and CPU, and only the first
  // one gets the children. Roots are processes whose parent has exited, as on a real system.
  const P = (name, mem, cpu = 0, kids = [], n = 1, share = 0) => ({ name, mem, cpu, kids, n, share });
  const TREE = [
    P("System", 0.2, 0.6, [P("Registry", 30), P("smss", 1), P("Memory Compression", 420), P("Secure System", 40)]),
    P("csrss", 6, 0.1, [], 2),
    P("wininit", 2, 0, [
      P("services", 8, 0, [
        P("svchost", 70, 0.2, [
          P("RuntimeBroker", 46, 0, [], 8), P("dllhost", 9, 0, [], 4), P("SearchHost", 160), P("StartMenuExperienceHost", 75),
          P("ShellExperienceHost", 48), P("TextInputHost", 36), P("ApplicationFrameHost", 22), P("WmiPrvSE", 18, 0, [], 3),
          P("smartscreen", 9), P("Widgets", 60, 0, [P("msedgewebview2", 180, 0, [], 5)]), P("WidgetService", 10),
          P("PhoneExperienceHost", 70), P("CrossDeviceService", 25), P("UserOOBEBroker", 3), P("CompPkgSrv", 3),
          P("unsecapp", 3), P("backgroundTaskHost", 10),
        ]),
        P("svchost", 40, 0.1, [P("sihost", 12), P("taskhostw", 10, 0, [], 2)]),
        P("svchost", 14, 0.2, [P("audiodg", 14, 0.2)]),
        P("svchost", 870, 0.4, [], 81),
        P("spoolsv", 7), P("MsMpEng", 280, 0.2, [], 1, 0.04), P("MpDefenderCoreService", 30), P("NisSrv", 5),
        P("SecurityHealthService", 6), P("NVDisplay.Container", 45, 0.1, [P("NVDisplay.Container", 20)]),
        P("nvcontainer", 38, 0, [P("nvcontainer", 30, 0, [], 2)]), P("SearchIndexer", 40, 0, [P("SearchProtocolHost", 3), P("SearchFilterHost", 2)]),
        P("WUDFHost", 6, 0, [], 3), P("sppsvc", 8), P("AggregatorHost", 3), P("dasHost", 4),
        P("postgres", 10, 0, [P("postgres", 60, 0, [], 7)]),
      ]),
      P("lsass", 12), P("fontdrvhost", 3), P("LsaIso", 2),
    ]),
    P("winlogon", 4, 0, [P("fontdrvhost", 2), P("dwm", 160, 1)]),
    P("explorer", 210, 0.2, [
      P("chrome", 380, 0.6, [P("chrome", 1970, 1.6, [], 23)]),
      P("Code", 420, 0.3, [P("Code", 1060, 0.7, [P("rust-analyzer", 1900, 0.3, [P("rust-analyzer-proc-macro-srv", 90)], 1, 0.05)], 13, 0.02)]),
      P("WindowsTerminal", 120, 0.1, [
        P("OpenConsole", 12, 0, [P("pwsh", 95, 0, [P("cargo", 46, 0.3)])]),
        P("OpenConsole", 8, 0, [P("pwsh", 80, 0, [P("node", 210, 0.1, [], 2)])]),
      ]),
      P("Discord", 140, 0.1, [P("Discord", 480, 0.2, [], 5)]),
      P("Spotify", 160, 0.2, [P("Spotify", 250, 0.3, [], 5)]),
      P("steam", 140, 0.1, [P("steamwebhelper", 120, 0.1, [P("steamwebhelper", 400, 0.2, [], 8)])]),
      P("ollama app", 30, 0, [P("ollama", 620, 0.1)]),
      P("Obsidian", 120, 0.05, [P("Obsidian", 260, 0.05, [], 4)]),
      P("1Password", 140, 0.05, [P("1Password", 120, 0.05, [], 3)]),
      P("Signal", 110, 0, [P("Signal", 150, 0, [], 3)]),
      P("PowerToys", 90, 0, [P("PowerToys.PowerLauncher", 140), P("PowerToys.FancyZones", 30), P("PowerToys.Awake", 18)]),
      P("Everything", 70), P("ShareX", 85), P("LibreHardwareMonitor", 95, 0.3),
      P("Docker Desktop", 120, 0.05, [P("Docker Desktop", 170, 0.05, [], 4), P("com.docker.backend", 120, 0.1, [], 2)]),
      P("OneDrive", 110), P("lghub", 180, 0.1, [P("lghub_agent", 40)]), P("SecurityHealthSystray", 2), P("ctfmon", 9),
      P("oriel", 11, 0.1, [P("msedgewebview2", 130, 0.2, [P("msedgewebview2", 90, 0.2, [], 5)])]),
    ]),
    P("msedge", 120, 0, [P("msedge", 90, 0, [], 5)]),
    P("vmmemWSL", 1800, 0.4),
  ];
  // Windows process ids are multiples of 4. A separate generator, so the graphs don't change with the tree.
  const pidRnd = prng(7);
  const used = new Set([0, 4]);
  const newPid = () => {
    let pid;
    do pid = 4 * (60 + Math.floor(pidRnd() * 9000)); while (used.has(pid));
    used.add(pid);
    return pid;
  };
  const LIVE = []; // {pid, ppid, node, first, w}
  let cargo = 0;
  (function plant(nodes, ppid) {
    for (const node of nodes) {
      for (let k = 0; k < node.n; k++) {
        const pid = node.name === "System" ? 4 : newPid();
        LIVE.push({ pid, ppid, node, first: k === 0, w: node.n > 1 ? 0.5 + pidRnd() : 1 });
        if (node.name === "cargo") cargo = pid;
        if (k === 0) plant(node.kids, pid);
      }
    }
  })(TREE, 0);
  const RUSTC = Array.from({ length: 8 }, newPid);
  const PROTECTED = new Set(["System", "Registry", "smss", "Memory Compression", "Secure System", "csrss", "wininit", "services",
    "svchost", "lsass", "LsaIso", "winlogon", "fontdrvhost", "dwm", "MsMpEng", "MpDefenderCoreService", "NisSrv", "SecurityHealthService"]);

  function procs(s) {
    const out = [];
    for (const { pid, ppid, node, first, w } of LIVE) {
      if (node.name === "cargo" && !phases.build) continue; // only there while building
      let cpu = (node.cpu / node.n) * (0.4 + rnd() * 1.2);
      if (phases.build && node.share && first) cpu += node.share * s.cpu.total;
      if (node.name === "ollama" && phases.gpu) cpu += 1.5 + rnd();
      out.push({ pid, ppid, name: node.name, cpu: r2(cpu), mem: Math.round((node.mem / node.n) * w * MB) });
    }
    if (phases.build) {
      const n = 3 + Math.floor(rnd() * 6);
      for (let k = 0; k < n; k++) {
        out.push({ pid: RUSTC[k], ppid: cargo, name: "rustc", cpu: r2(((s.cpu.total * 0.82) / n) * (0.7 + rnd() * 0.6)), mem: Math.round((380 + rnd() * 300) * MB) });
      }
    }
    return out;
  }
  const DRIVES = [{ mount: "C:", used: 611 * GB, total: 930.6 * GB }, { mount: "D:", used: 1710 * GB, total: 1863 * GB }];

  // Now playing: made-up tracks with drawn covers. ?media=off for nothing playing.
  const TRACKS = [
    { title: "Glass Orchard", artist: "Northern Static", album: "Low Light Hours", len: 247, hue: 262 },
    { title: "Paper Satellites", artist: "Mira Vale", album: "Small Weather", len: 213, hue: 196 },
    { title: "Overpass at 4 AM", artist: "The Quiet Engines", album: "Transit", len: 289, hue: 18 },
  ];
  const cover = h => "data:image/svg+xml," + encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">` +
    `<stop offset="0" stop-color="hsl(${h} 70% 62%)"/><stop offset="1" stop-color="hsl(${h + 50} 55% 26%)"/></linearGradient></defs>` +
    `<rect width="100" height="100" fill="url(#g)"/><circle cx="68" cy="34" r="15" fill="hsl(${h + 25} 90% 86%)" opacity=".85"/>` +
    `<path d="M0 76Q28 56 54 70T100 62V100H0Z" fill="hsl(${h + 60} 50% 16%)" opacity=".75"/></svg>`);
  const player = { track: 0, pos: 96, playing: true, at: Date.now() / 1000 };
  function media() {
    if (q.get("media") === "off") return undefined;
    const now = Date.now() / 1000;
    if (player.playing) player.pos += now - player.at;
    player.at = now;
    let tr = TRACKS[player.track];
    if (player.pos >= tr.len) { player.track = (player.track + 1) % TRACKS.length; player.pos = 0; tr = TRACKS[player.track]; }
    return { app: "Spotify.exe", title: tr.title, artist: tr.artist, album: tr.album, playing: player.playing,
      position: player.pos, duration: tr.len, at: now, can_prev: true, can_next: true, can_toggle: true, art: player.track + 1 };
  }
  function control(action) {
    media(); // bring the position up to now first
    if (action === "toggle") player.playing = !player.playing;
    else if (action === "previous" && player.pos > 3) player.pos = 0;
    else {
      player.track = (player.track + (action === "next" ? 1 : TRACKS.length - 1)) % TRACKS.length;
      player.pos = 0;
    }
  }

  // Volume mixer: a few apps on Windows' generic speakers. Spotify is active while it plays.
  const mixer = {
    device: "Speakers (High Definition Audio Device)", volume: 62, muted: false,
    apps: [
      { pid: 8412, exe: "Spotify", title: "", volume: 80, muted: false },
      { pid: 5120, exe: "chrome", title: "", volume: 100, muted: false },
      { pid: 9734, exe: "Discord", title: "", volume: 65, muted: true },
      { pid: 0, exe: "", title: "", volume: 50, muted: false },
    ],
  };
  const audio = () => q.get("audio") === "off" ? undefined
    : { ...mixer, apps: mixer.apps.map(a => ({ ...a, active: a.pid === 8412 && player.playing })) };

  // Open windows, top-most first. Made-up programs and titles; clicking one brings it to the front.
  const WINDOWS = [
    { hwnd: 1001, pid: 4120, exe: "Code", title: "layout.rs - orbit-notes - Visual Studio Code" },
    { hwnd: 1002, pid: 5120, exe: "chrome", title: "Trail conditions, Saturday - Google Chrome" },
    { hwnd: 1003, pid: 6644, exe: "WindowsTerminal", title: "cargo watch -x test" },
    { hwnd: 1004, pid: 8412, exe: "Spotify", title: "Spotify Premium" },
    { hwnd: 1005, pid: 9734, exe: "Discord", title: "#general | Lantern Club - Discord", minimized: true },
    { hwnd: 1006, pid: 3320, exe: "explorer", title: "Photos" },
    { hwnd: 1007, pid: 7210, exe: "notepad", title: "shopping list.txt - Notepad" },
    { hwnd: 1008, pid: 2788, exe: "EXCEL", title: "Budget 2027.xlsx - Excel", minimized: true },
    { hwnd: 1009, pid: 5530, exe: "ApplicationFrameHost", title: "Calculator" },
    { hwnd: 1010, pid: 6102, exe: "Fieldnotes", title: "Untitled sketch - Fieldnotes" },
  ];
  let frontWin = 1001;
  const windows = () => {
    if (q.get("windows") === "none") return [];
    let list = WINDOWS;
    if (q.get("windows") === "many") {
      list = Array.from({ length: 30 }, (_, i) => ({ ...WINDOWS[i % WINDOWS.length], hwnd: 2000 + i, title: `${WINDOWS[i % WINDOWS.length].title} (${i + 1})` }));
    }
    const front = q.get("windows") === "front" ? list.find(w => !w.minimized)?.hwnd : frontWin;
    return list.map(w => ({ minimized: false, ...w, front: w.hwnd === front }));
  };

  // Back-date the history so it ends now, one sample per tick.
  const T0 = Date.now() / 1000;
  const history = Array.from({ length: 300 }, (_, k) => ({ ...step(), t: T0 - (300 - k) }));
  const full = () => {
    const s = step();
    return { ...s, procs: procs(s), drives: DRIVES, media: media(), audio: audio(), windows: windows() };
  };

  const listeners = {};
  const emit = () => listeners.sample && listeners.sample({ payload: full() });
  let first = true; // click= and settings= act on the first start only, not on every rebuild
  window.__TAURI__ = {
    core: {
      async invoke(cmd, args) {
        switch (cmd) {
          case "hello":
            // History has no process list, so send a full sample straight away.
            setTimeout(() => {
              emit();
              if (!first) return;
              first = false;
              for (const name of (q.get("click") || "").split(",").filter(Boolean)) {
                const rows = [...document.querySelectorAll(".procs .r.x")];
                rows.find(r => r.title.split(" · ")[0] === name)?.click();
              }
              if (q.get("settings") === "1") Oriel.settingsPanel.open();
            }, 0);
            return {
              info, interval_ms: settings.interval_ms, history, settings, autostart: true,
              monitors: [{ index: 1, x: 0, y: 0, width: 2560, height: 1440, primary: true }, { index: 2, x: 2560, y: 0, width: 1920, height: 1080, primary: false }],
              settings_path: "%APPDATA%\\Oriel\\settings.json",
            };
          case "save_settings": {
            settings = structuredClone(args.settings);
            // As settings.rs tidies page names.
            settings.pages = settings.pages.slice(0, 9).map((p, i) => ({ ...p, name: p.name.trim().slice(0, 32).trim() || `Page ${i + 1}` }));
            return settings;
          }
          case "set_autostart": return args.enabled;
          case "media_art": return cover(TRACKS[player.track].hue);
          case "media_control": control(args.action); setTimeout(emit, 150); return null;
          case "set_volume": {
            const to = args.pid == null ? mixer : mixer.apps.find(a => a.pid === args.pid);
            if (to && args.volume != null) to.volume = Math.round(args.volume);
            if (to && args.muted != null) to.muted = args.muted;
            return null;
          }
          case "focus_window": {
            const w = WINDOWS.find(w => w.hwnd === args.hwnd);
            if (!w || w.pid !== args.pid) throw "that window has gone";
            w.minimized = false;
            frontWin = w.hwnd;
            setTimeout(emit, 100);
            return null;
          }
          case "close_window": {
            const i = WINDOWS.findIndex(w => w.hwnd === args.hwnd && w.pid === args.pid);
            if (i < 0) throw "that window has gone";
            WINDOWS.splice(i, 1);
            if (frontWin === args.hwnd) frontWin = WINDOWS[0]?.hwnd;
            setTimeout(emit, 100);
            return null;
          }
          case "end_processes": {
            // As on a real PC: Windows' own processes refuse, and Oriel won't end itself.
            const r = { ended: 0, denied: 0, gone: 0, own: false };
            for (const pid of args.pids) {
              const i = LIVE.findIndex(p => p.pid === pid);
              const name = LIVE[i]?.node.name;
              if (i < 0) r.gone++;
              else if (name === "oriel") r.own = true;
              else if (PROTECTED.has(name)) r.denied++;
              else { LIVE.splice(i, 1); r.ended++; }
            }
            return r;
          }
          default: return null; // window and Windows actions do nothing here
        }
      },
    },
    event: { async listen(name, cb) { listeners[name] = cb; return () => {}; } },
  };
  setInterval(emit, settings.interval_ms);
})();
