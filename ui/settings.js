"use strict";
// The settings drawer. Every change saves straight away; there is no Apply button.
{
const O = Oriel;
const { $, el, esc } = O;
const { invoke } = window.__TAURI__.core;

const DEFAULT_LAYOUT = [["cpu", "gpu", "memory"], ["network", "disk", "processes"]];
const GRAPH = [[60, "1 minute"], [120, "2 minutes"], [300, "5 minutes"], [600, "10 minutes"]];
const INTERVAL = [[500, "0.5 seconds"], [1000, "1 second"], [2000, "2 seconds"], [5000, "5 seconds"]];
const LIMITS = [["cpu_temp", "CPU temperature", "°C"], ["gpu_temp", "GPU temperature", "°C"], ["mem", "Memory used", "%"], ["drive", "Drive full", "%"]];
// Palettes are defined in themes.css; this is the order and the names they're shown with.
const PALETTES = [["oriel", "Oriel"], ["catppuccin", "Catppuccin"], ["nord", "Nord"], ["gruvbox", "Gruvbox"], ["rose-pine", "Rosé Pine"], ["solarized", "Solarized"]];
// Changing these means re-reading history at a new resolution, so the dashboard is rebuilt.
const REBUILD = new Set(["graph_seconds", "interval_ms", "layout"]);
// Read once at startup (sensor threads, WebView2 flags).
const RESTART = new Set(["nvml", "lhm_url", "gpu"]);

const drawer = () => $("#settings");
const searchInput = () => drawer().querySelector(".search input");
const S = () => O.state.settings;
let needsRestart = false;

function debounce(fn, ms) {
  let t = 0;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

async function save(patch) {
  const next = { ...S(), ...patch };
  O.state.settings = next; // show it now; the backend's clamped copy replaces it below
  try {
    O.state.settings = await invoke("save_settings", { settings: next });
  } catch (e) {
    console.warn("couldn't save settings", e);
  }
  const keys = Object.keys(patch);
  if (keys.some(k => RESTART.has(k))) { needsRestart = true; syncRestart(); }
  if (keys.some(k => REBUILD.has(k))) await O.reload();
  else O.applySettings();
}

const options = (list, value) => list.map(([v, label]) => `<option value="${v}"${String(v) === String(value) ? " selected" : ""}>${esc(label)}</option>`).join("");
const sw = (id, key, checked) => `<input id="${id}" type="checkbox" class="switch" data-set="${key}"${checked ? " checked" : ""}>`;

function render() {
  const s = S();
  const { monitors, settingsPath, info } = O.state;
  const monitorOpts = [["auto", "Automatic"], ...monitors.map(m => [m.index, `Display ${m.index} · ${m.width}×${m.height}${m.primary ? " (main)" : ""}`])];
  // Search: each element with data-s is one result; its text plus the data-s words are what's searched.
  // A section's data-sk words (and its title) match the whole section.
  drawer().innerHTML = `
    <header><h2>Settings</h2>
      <button class="icon" type="button" data-close title="Close (Esc)" aria-label="Close settings">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg></button></header>
    <div class="search">${O.findBox("Search settings", "Search settings")}</div>
    <div class="body">
      <section data-sk="look style">
        <h3>Appearance</h3>
        <div class="field" data-s="theme dark light auto night day color scheme windows"><span>Mode</span>
          <span class="seg" role="group" aria-label="Mode" data-seg="theme">
            <button type="button" data-v="dark">Dark</button><button type="button" data-v="light">Light</button><button type="button" data-v="auto">Auto</button>
          </span></div>
        <div class="stack" data-s="theme palette colors colours accent">
          <span>Theme</span>
          <div class="palettes" role="group" aria-label="Theme">
            ${PALETTES.map(([id, name]) => `
              <button type="button" class="pal" data-palette="${id}" data-mode="${O.state.mode}" data-v="${id}">
                <span class="pv"><i></i><i></i><i></i><i></i></span><span class="pn">${esc(name)}</span></button>`).join("")}
          </div></div>
        <div class="field" data-s="chart graph time window minutes length"><label for="s-graph">Graph history</label>
          <select id="s-graph" data-set="graph_seconds" data-num>${options(GRAPH, s.graph_seconds)}</select></div>
        <div class="field" data-s="oled pixel shift screen saver"><label for="s-burn">Burn-in protection<span class="hint">Shifts everything a pixel or two every few minutes, for OLED screens</span></label>
          ${sw("s-burn", "burn_in_shift", s.burn_in_shift)}</div>
      </section>

      <section>
        <h3>Layout</h3>
        <div class="stack" data-s="widgets tiles rows order arrange move add remove hide show reset drag">
          <p class="note">Drag widgets to reorder them or move them between rows. Widgets in a row share its width.</p>
          <div class="rows" id="s-rows"></div>
          <div class="actions"><button class="btn" type="button" data-add-row>Add row</button>
            <button class="btn link" type="button" data-reset-layout>Reset to default</button></div></div>
      </section>

      <section data-sk="monitor screen">
        <h3>Display</h3>
        <div class="field" data-s="monitor screen display which"><label for="s-monitor">Show on<span class="hint">Automatic picks a screen other than your main one</span></label>
          <select id="s-monitor" data-set="monitor">${options(monitorOpts, s.monitor)}</select></div>
        <div class="field" data-s="fullscreen window maximize f11"><label for="s-full">Full screen<span class="hint">Not used when Oriel would cover your only screen. <kbd>F11</kbd> toggles it.</span></label>
          ${sw("s-full", "fullscreen", s.fullscreen)}</div>
        <div class="actions" data-s="resolution scale scaling monitor screen"><button class="btn link" type="button" data-action="ms-settings:display">Windows display settings</button></div>
      </section>

      <section data-sk="performance">
        <h3>System</h3>
        <div class="field" data-s="autostart startup login boot launch windows"><label for="s-auto">Start at sign-in</label>
          <input id="s-auto" type="checkbox" class="switch" data-autostart${O.state.autostart ? " checked" : ""}></div>
        <div class="field" data-s="interval refresh rate frequency speed seconds performance"><label for="s-interval">Update every<span class="hint">More often costs a little more CPU</span></label>
          <select id="s-interval" data-set="interval_ms" data-num>${options(INTERVAL, s.interval_ms)}</select></div>
        <div class="field" data-s="gpu webview2 performance ram"><label for="s-gpu">Hardware acceleration<span class="hint">Draw with the GPU: slightly less CPU, about 130 MB more memory. Applies after a restart.</span></label>
          ${sw("s-gpu", "gpu", s.gpu)}</div>
        <div class="actions" data-restart-row hidden><span class="note">Restart Oriel to apply.</span>
          <button class="btn primary" type="button" data-restart>Restart now</button></div>
      </section>

      <section data-sk="warnings thresholds limits">
        <h3>Alerts</h3>
        <p class="note">Readings get an amber “warm/high” label at the first value and a red one at the second.</p>
        <div class="limits">
          <span></span><span class="head">Warn</span><span class="head">Critical</span>
          ${LIMITS.map(([key, label, unit]) => `
            <div class="lim" data-s="alert warn warning critical threshold limit hot high full">
            <span>${label}</span>
            <label><input type="number" min="1" max="150" step="1" data-limit="${key}" data-i="0" value="${s.limits[key][0]}" aria-label="${label} warning">${unit}</label>
            <label><input type="number" min="1" max="150" step="1" data-limit="${key}" data-i="1" value="${s.limits[key][1]}" aria-label="${label} critical">${unit}</label></div>`).join("")}
        </div>
      </section>

      <section data-sk="hardware">
        <h3>Sensors</h3>
        <div class="field" data-s="gpu geforce nvml temperature power clocks laptop"><label for="s-nvml">NVIDIA sensors<span class="hint">Temperature, power and clocks for GeForce cards. A sleeping laptop GPU is never woken to read them.</span></label>
          ${sw("s-nvml", "nvml", s.nvml)}</div>
        <div data-s="lhm cpu temperature fans power sensor url"><label for="s-lhm">LibreHardwareMonitor address</label>
          <span class="hint">Windows doesn't let apps read the CPU's own temperature sensor or fan speeds. LibreHardwareMonitor can: run it and turn on Options › Remote Web Server. Leave empty to turn this off.</span>
          <input id="s-lhm" type="text" spellcheck="false" data-text="lhm_url" value="${esc(s.lhm_url)}" placeholder="http://127.0.0.1:8085/data.json" style="margin-top:0.5rem"></div>
        <div class="actions" data-restart-row hidden><span class="note">Restart Oriel to apply.</span>
          <button class="btn primary" type="button" data-restart>Restart now</button></div>
      </section>

      <section>
        <h3>About</h3>
        <p class="note" data-s="version license">Oriel ${esc(info.version)} · MIT License</p>
        <p class="note" data-s="file json location folder appdata">Settings are saved to <code>${esc(settingsPath)}</code></p>
        <p class="note" data-s="keyboard shortcuts keys hotkeys"><kbd>,</kbd> settings &nbsp;·&nbsp; <kbd>/</kbd> search &nbsp;·&nbsp; <kbd>Esc</kbd> close &nbsp;·&nbsp; <kbd>F11</kbd> full screen</p>
      </section>
      <p class="none" data-none hidden></p>
    </div>`;
  syncTheme();
  syncRestart();
  renderRows();
}

function syncTheme() {
  const s = S();
  drawer().querySelectorAll('[data-seg="theme"] button').forEach(b => b.setAttribute("aria-pressed", String(b.dataset.v === s.theme)));
  drawer().querySelectorAll(".pal").forEach(b => {
    b.setAttribute("aria-pressed", String(b.dataset.v === (s.palette || "oriel")));
    b.dataset.mode = O.state.mode; // previews show the variant for the current mode
  });
}

// ---- search ------------------------------------------------------------------------
function search(query) {
  const body = drawer().querySelector(".body");
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const has = (text, extra = "") => {
    const hay = `${text} ${extra}`.toLowerCase();
    return words.every(w => hay.includes(w));
  };
  let found = 0;
  for (const sec of body.querySelectorAll("section")) {
    const whole = !words.length || has(sec.querySelector("h3").textContent, sec.dataset.sk);
    const items = [...sec.querySelectorAll("[data-s]")];
    let n = 0;
    for (const it of items) {
      const ok = whole || has(it.textContent, it.dataset.s);
      it.classList.toggle("nomatch", !ok);
      n += ok;
    }
    // Everything else in a section (notes, headings, containers) shows when something in it does.
    for (const c of sec.children) {
      if (c.tagName === "H3" || "s" in c.dataset) continue;
      const inner = c.querySelectorAll("[data-s]");
      c.classList.toggle("nomatch", inner.length ? ![...inner].some(i => !i.classList.contains("nomatch")) : !n);
    }
    sec.classList.toggle("nomatch", !n);
    found += n;
  }
  const none = body.querySelector("[data-none]");
  none.hidden = !!found;
  none.textContent = found ? "" : `No settings match “${query.trim()}”.`;
  mark(body, words);
}

/** Highlight the matched words without touching the DOM (CSS Custom Highlight API). */
function mark(body, words) {
  if (!window.Highlight || !CSS.highlights) return;
  const hl = new Highlight();
  if (words.length) {
    const walk = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
    for (let node; (node = walk.nextNode()); ) {
      if (node.parentElement.closest(".nomatch, select")) continue;
      const text = node.data.toLowerCase();
      for (const w of words) {
        for (let i = text.indexOf(w); i >= 0; i = text.indexOf(w, i + w.length)) {
          const r = new Range();
          r.setStart(node, i);
          r.setEnd(node, i + w.length);
          hl.add(r);
        }
      }
    }
  }
  CSS.highlights.set("settings-search", hl);
}
function syncRestart() {
  drawer().querySelectorAll("[data-restart-row]").forEach(row => (row.hidden = !needsRestart));
}

// ---- layout editor ---------------------------------------------------------------
function renderRows() {
  const box = $("#s-rows");
  if (!box) return;
  const layout = S().layout;
  const used = new Set(layout.flat());
  const free = [...O.widgets.values()].filter(w => !used.has(w.id));
  box.replaceChildren(...layout.map((ids, r) => {
    const row = el(`<div class="lrow" data-row="${r}"><span class="n">${r + 1}</span></div>`);
    for (const id of ids) {
      const def = O.widgets.get(id);
      const name = def ? def.name : `${id} (missing)`;
      row.appendChild(el(`
        <span class="wchip" draggable="true" data-id="${esc(id)}" style="--c:var(--${def ? def.color : "muted"})" title="${esc(def ? def.description : "This widget isn't installed")}">
          ${esc(name)}<button type="button" data-remove aria-label="Remove ${esc(name)}">×</button></span>`));
    }
    if (!ids.length) row.appendChild(el(`<span class="placeholder">Empty: drop a widget here</span>`));
    if (free.length) {
      row.appendChild(el(`<select class="add" aria-label="Add a widget to row ${r + 1}"><option value="">Add…</option>
        ${free.map(w => `<option value="${esc(w.id)}">${esc(w.name)}</option>`).join("")}</select>`));
    }
    return row;
  }));
  const q = searchInput();
  if (q?.value) search(q.value); // the new chips need filtering and marking too
}

function setLayout(layout) {
  save({ layout }).then(renderRows);
  renderRows();
}

let dragging = null;
function wireRows(box) {
  box.addEventListener("dragstart", e => {
    const chip = e.target.closest(".wchip");
    if (!chip) return;
    dragging = chip.dataset.id;
    chip.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", dragging);
  });
  box.addEventListener("dragend", () => {
    dragging = null;
    box.querySelectorAll(".dragging, .over").forEach(n => n.classList.remove("dragging", "over"));
  });
  box.addEventListener("dragover", e => {
    const row = e.target.closest(".lrow");
    if (!row || dragging == null) return;
    e.preventDefault();
    box.querySelectorAll(".lrow.over").forEach(n => n !== row && n.classList.remove("over"));
    row.classList.add("over");
  });
  box.addEventListener("drop", e => {
    const row = e.target.closest(".lrow");
    if (!row || dragging == null) return;
    e.preventDefault();
    const r = Number(row.dataset.row);
    // Insert before the first chip that's right of (or below) the pointer.
    const chips = [...row.querySelectorAll(".wchip")].filter(c => c.dataset.id !== dragging);
    let at = chips.findIndex(c => {
      const b = c.getBoundingClientRect();
      return e.clientY < b.top || (e.clientY <= b.bottom && e.clientX < b.left + b.width / 2);
    });
    if (at < 0) at = chips.length;
    const layout = S().layout.map(ids => ids.filter(id => id !== dragging));
    layout[r].splice(at, 0, dragging);
    setLayout(layout);
  });
}

// ---- events ------------------------------------------------------------------------
const saveLimits = debounce(input => {
  const key = input.dataset.limit;
  const [warn, crit] = [...drawer().querySelectorAll(`[data-limit="${key}"]`)].map(n => Number(n.value));
  const bad = !(warn > 0 && crit > 0) || warn >= crit;
  drawer().querySelectorAll(`[data-limit="${key}"]`).forEach(n => n.setCustomValidity(bad ? "Warn must be below critical" : ""));
  if (!bad) save({ limits: { ...S().limits, [key]: [warn, crit] } });
}, 400);
const saveText = debounce(input => save({ [input.dataset.text]: input.value.trim() }), 600);

function wire() {
  const d = drawer();
  d.addEventListener("click", e => {
    const t = e.target;
    if (t.closest("[data-close]")) return close();
    const seg = t.closest("[data-seg] button");
    if (seg) {
      save({ [seg.parentElement.dataset.seg]: seg.dataset.v });
      return syncTheme();
    }
    const pal = t.closest(".pal");
    if (pal) {
      save({ palette: pal.dataset.v });
      return syncTheme();
    }
    if (t.closest("[data-remove]")) {
      const id = t.closest(".wchip").dataset.id;
      return setLayout(S().layout.map(ids => ids.filter(x => x !== id)));
    }
    if (t.closest("[data-add-row]")) return setLayout([...S().layout, []]);
    if (t.closest("[data-reset-layout]")) return setLayout(DEFAULT_LAYOUT.map(r => [...r]));
    if (t.closest("[data-restart]")) invoke("restart");
  });
  d.addEventListener("change", async e => {
    const t = e.target;
    if (t.matches("select.add") && t.value) {
      const r = Number(t.closest(".lrow").dataset.row);
      return setLayout(S().layout.map((ids, i) => (i === r ? [...ids, t.value] : ids)));
    }
    if (t.matches("[data-autostart]")) {
      try {
        O.state.autostart = await invoke("set_autostart", { enabled: t.checked });
      } catch (err) {
        console.warn(err);
      }
      t.checked = O.state.autostart;
      return;
    }
    const key = t.dataset.set;
    if (!key) return;
    const value = t.type === "checkbox" ? t.checked : "num" in t.dataset ? Number(t.value) : t.value;
    save({ [key]: value });
  });
  d.addEventListener("input", e => {
    if (e.target.dataset.limit) saveLimits(e.target);
    else if (e.target.dataset.text) saveText(e.target);
    else if (e.target === searchInput()) search(e.target.value);
  });
  // Esc in a non-empty search clears it; the next Esc closes the panel.
  d.addEventListener("keydown", e => {
    const q = searchInput();
    if (e.key !== "Escape" || e.target !== q || !q.value) return;
    e.preventDefault();
    e.stopPropagation();
    q.value = "";
    search("");
  });
  $("#scrim").addEventListener("click", close);
}

// ---- open / close --------------------------------------------------------------------
const isOpen = () => drawer().classList.contains("open");
function open() {
  render();
  wireRows($("#s-rows"));
  drawer().inert = false;
  drawer().classList.add("open");
  $("#scrim").classList.add("open");
  search("");
  searchInput().focus(); // so typing straight away searches
}
function close() {
  if (!isOpen()) return;
  drawer().classList.remove("open");
  drawer().inert = true;
  $("#scrim").classList.remove("open");
  O.hideTip();
  // Empty rows are kept while editing so there's somewhere to drop; tidy them away now.
  const layout = S().layout;
  if (layout.some(r => !r.length)) save({ layout: layout.filter(r => r.length) });
}

O.settingsPanel = {
  open,
  close,
  isOpen,
  toggle: () => (isOpen() ? close() : open()),
  /** Keep controls in step with changes made elsewhere (header theme button, tray menu). */
  sync() {
    if (!isOpen()) return;
    syncTheme();
    const a = drawer().querySelector("[data-autostart]");
    if (a) a.checked = O.state.autostart;
  },
};
wire();
}
