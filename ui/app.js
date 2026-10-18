"use strict";
// Boot, layout and the sample loop. The backend pushes a "sample" event each tick while the window is visible.
{
const O = Oriel;
const { $, el, setText, setHTML, fmt } = O;
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let mounted = [];   // every tile on the page: {root, update}
let latest = null;  // newest sample, re-rendered when thresholds change
let pending = null; // newest sample that arrived while the page was hidden
let lastAt = 0;
let shiftTimer = 0;

async function boot() {
  const hello = await invoke("hello");
  Object.assign(O.state, {
    info: hello.info,
    settings: hello.settings,
    interval: hello.interval_ms / 1000,
    monitors: hello.monitors,
    autostart: hello.autostart,
    settingsPath: hello.settings_path,
  });
  O.applySettings();
  build(hello.history);
}
O.reload = boot;

const prefersLight = matchMedia("(prefers-color-scheme: light)");

/** Settings that don't need a rebuild: theme, burn-in shift, thresholds. */
O.applySettings = () => {
  const s = O.state.settings;
  const root = document.documentElement;
  O.state.mode = s.theme === "auto" ? (prefersLight.matches ? "light" : "dark") : s.theme === "light" ? "light" : "dark";
  root.dataset.mode = O.state.mode;
  root.dataset.palette = s.palette || "oriel";
  O.readTokens();
  themeButton(s.theme);
  O.settingsPanel.sync();

  clearInterval(shiftTimer);
  $("#stage").style.transform = "";
  if (s.burn_in_shift) {
    // Nudge everything a pixel or two every few minutes so static UI can't burn into OLED panels.
    shiftTimer = setInterval(() => {
      const r = () => Math.round(Math.random() * 4 - 2);
      $("#stage").style.transform = `translate(${r()}px, ${r()}px)`;
    }, 180000);
  }
  if (latest) render(latest);
  O.drawCharts();
};

// ---- layout ------------------------------------------------------------------------
function build(history) {
  const { info, settings, interval } = O.state;
  const grid = $("#grid");
  grid.replaceChildren();
  O.charts.length = 0;
  mounted = [];
  const cap = Math.round(settings.graph_seconds / interval) + 1;
  const rows = [], rowsPortrait = [];

  for (const ids of settings.layout) {
    const row = el(`<section class="row"></section>`);
    const cols = [];
    let h = 0;
    for (const id of ids) {
      const def = O.widgets.get(id);
      if (!def) continue;
      for (const inst of def.instances ? def.instances(info) : [undefined]) {
        const ctx = {
          info,
          inst,
          go: O.go,
          chart(target, opts) {
            const c = new O.Chart(target, opts);
            c.cap = cap;
            O.charts.push(c);
            return c;
          },
        };
        try {
          const w = def.create(ctx);
          w.root.dataset.widget = id;
          row.appendChild(w.root);
          mounted.push(w);
          cols.push(`${def.size.w}fr`);
          h = Math.max(h, def.size.h);
        } catch (e) {
          console.error(`widget ${id} failed to start`, e);
        }
      }
    }
    if (!cols.length) continue;
    row.style.setProperty("--cols", cols.join(" "));
    grid.appendChild(row);
    rows.push(`${h}fr`);
    rowsPortrait.push(`${h * Math.ceil(cols.length / 2)}fr`); // portrait wraps rows into two columns
  }
  if (!rows.length) {
    grid.appendChild(el(`<p class="empty">Nothing on the dashboard yet.<br><button type="button" data-open-settings>Open settings</button> to add widgets.</p>`));
  }
  grid.style.setProperty("--rows", rows.join(" ") || "1fr");
  grid.style.setProperty("--rows-p", rowsPortrait.join(" ") || "1fr");

  for (const s of history.slice(-cap)) for (const c of O.charts) c.push(s);
  // History samples are slim (no process list or drives); the next tick fills those in.
  if (history.length) render(history[history.length - 1]);
  O.drawCharts();

  $("#host").textContent = info.host;
  $("#os").textContent = info.os;
  document.title = `${info.host} · Oriel`;
}

// ---- per-sample rendering -------------------------------------------------------------
function render(s) {
  latest = s;
  for (const w of mounted) {
    try {
      w.update(s);
    } catch (e) {
      console.error(`widget ${w.root.dataset.widget} failed to update`, e);
    }
  }
  renderBattery(s.battery);
  setText($("#uptime"), `up ${fmt.dur(s.uptime)}`);
}

function renderBattery(b) {
  const node = $("#battery");
  node.hidden = !b;
  if (!b) return;
  const low = !b.plugged && b.pct <= 10 ? "crit" : !b.plugged && b.pct <= 20 ? "warn" : "";
  const fill = low ? `var(--${low})` : "var(--ink-2)";
  const status = b.plugged ? (b.pct >= 99 ? "Full" : "Plugged in") : b.secs_left ? `${fmt.dur(b.secs_left)} left` : "On battery";
  setHTML(node, `
    <svg viewBox="0 0 26 14" aria-hidden="true"><rect x="0.75" y="0.75" width="22" height="12.5" rx="3" fill="none" stroke="var(--muted)" stroke-width="1.5"/>
    <rect x="23.5" y="4.5" width="2" height="5" rx="1" fill="var(--muted)"/>
    <rect x="2.75" y="2.75" width="${(18 * b.pct) / 100}" height="8.5" rx="1.5" fill="${fill}"/></svg>
    <span>${b.pct}%</span><span style="color:var(--muted)">${status}</span>
    ${low ? `<span class="chip ${low}">Low battery</span>` : ""}`);
}

// ---- header ------------------------------------------------------------------------
const THEMES = ["dark", "light", "auto"];
const THEME_ICON = {
  dark: `<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z"/>`,
  light: `<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>`,
  auto: `<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5v17A8.5 8.5 0 0 0 12 3.5Z" fill="currentColor"/>`,
};
const THEME_NAME = { dark: "Dark mode", light: "Light mode", auto: "Auto: follows Windows" };
function themeButton(theme) {
  const b = $("#themeBtn");
  const next = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length];
  setHTML(b, `<svg viewBox="0 0 24 24" aria-hidden="true">${THEME_ICON[theme] || THEME_ICON.dark}</svg>`);
  b.title = `${THEME_NAME[theme] || THEME_NAME.dark}. Click for ${next}.`;
  b.setAttribute("aria-label", b.title);
}
$("#themeBtn").addEventListener("click", async () => {
  const theme = THEMES[(THEMES.indexOf(O.state.settings.theme) + 1) % THEMES.length];
  O.state.settings = await invoke("save_settings", { settings: { ...O.state.settings, theme } });
  O.applySettings();
});
$("#gearBtn").addEventListener("click", () => O.settingsPanel.toggle());
prefersLight.addEventListener("change", () => {
  if (O.state.settings?.theme === "auto") O.applySettings();
});

function tick() {
  const now = new Date();
  setText($("#time"), now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
  setText($("#date"), now.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }));
  const stale = !!lastAt && Date.now() - lastAt > Math.max(5000, O.state.interval * 5000);
  if (stale !== document.body.classList.contains("stale")) {
    document.body.classList.toggle("stale", stale);
    $("#conn").hidden = !stale;
  }
}
setInterval(tick, 1000);
tick();

// ---- input -------------------------------------------------------------------------
document.addEventListener("click", e => {
  const a = e.target.closest("[data-action]");
  if (a) O.action(a.dataset.action);
  else if (e.target.closest("[data-open-settings]")) O.settingsPanel.open();
});
document.addEventListener("keydown", e => {
  if (e.key === "Escape") return O.settingsPanel.close();
  if (e.key === "F11") {
    e.preventDefault();
    return invoke("toggle_fullscreen");
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
    e.preventDefault();
    return focusSearch();
  }
  if (e.target.closest("input, select, textarea")) return;
  if (e.key === ",") O.settingsPanel.toggle();
  else if (e.key === "/") {
    e.preventDefault();
    focusSearch();
  }
});
/** "/" or Ctrl+F: search the settings when they're open, else the process list. */
function focusSearch() {
  const input = O.settingsPanel.isOpen() ? $("#settings .find input") : $(".procs .find input");
  if (!input) return;
  input.focus();
  input.select();
}
// It's an app, not a web page: no Back/Reload/Print menu, except in text fields.
document.addEventListener("contextmenu", e => {
  if (!e.target.closest("input, textarea")) e.preventDefault();
});
addEventListener("resize", () => O.readTokens()); // rem follows the viewport, charts use it for labels

// ---- backend events ----------------------------------------------------------------
listen("sample", ({ payload: s }) => {
  if (!O.state.info) return;
  for (const c of O.charts) c.push(s);
  lastAt = Date.now();
  if (document.hidden) {
    pending = s; // minimised: keep the graphs' data, skip the drawing
    return;
  }
  render(s);
  O.drawCharts();
});
document.addEventListener("visibilitychange", () => {
  if (document.hidden) return;
  if (pending) render(pending);
  pending = null;
  O.drawCharts();
});
// Shown again from the tray: the page got nothing while hidden, so reload history.
listen("resync", () => boot());
listen("open-settings", () => O.settingsPanel.open());
listen("autostart", ({ payload }) => {
  O.state.autostart = payload;
  O.settingsPanel.sync();
});

boot().catch(e => {
  console.error(e);
  $("#grid").replaceChildren(el(`<p class="empty">Oriel couldn't start its sensors.<br>${O.esc(e)}</p>`));
});
}
