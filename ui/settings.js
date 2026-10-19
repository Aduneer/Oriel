"use strict";
// The settings drawer. Every change saves straight away; there is no Apply button.
{
const O = Oriel;
const { $, el, esc } = O;
const { invoke } = window.__TAURI__.core;

const DEFAULT_LAYOUT = [["cpu", "gpu", "memory"], ["network", "disk", "processes"]];
const MAX_PAGES = 9, MAX_NAME = 32; // as in settings.rs
const GRAPH = [[60, "1 minute"], [120, "2 minutes"], [300, "5 minutes"], [600, "10 minutes"]];
const INTERVAL = [[500, "0.5 seconds"], [1000, "1 second"], [2000, "2 seconds"], [5000, "5 seconds"]];
const ROTATE = [[0, "Off"], [10, "Every 10 seconds"], [30, "Every 30 seconds"], [60, "Every minute"], [300, "Every 5 minutes"]];
const LIMITS = [["cpu_temp", "CPU temperature", "°C"], ["gpu_temp", "GPU temperature", "°C"], ["mem", "Memory used", "%"], ["drive", "Drive full", "%"]];
// Palettes are defined in themes.css; this is the order and the names they're shown with.
const PALETTES = [["oriel", "Oriel"], ["catppuccin", "Catppuccin"], ["nord", "Nord"], ["gruvbox", "Gruvbox"], ["rose-pine", "Rosé Pine"], ["solarized", "Solarized"]];
// Changing these means re-reading history at a new resolution, so the dashboard is rebuilt.
const REBUILD = new Set(["graph_seconds", "interval_ms", "pages"]);
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

const layouts = s => JSON.stringify(s.pages.map(p => p.layout));

async function save(patch) {
  const before = S();
  const next = { ...before, ...patch };
  O.state.settings = next; // show it now; the backend's clamped copy replaces it below
  try {
    O.state.settings = await invoke("save_settings", { settings: next });
  } catch (e) {
    console.warn("couldn't save settings", e);
  }
  const keys = Object.keys(patch);
  if (keys.some(k => RESTART.has(k))) { needsRestart = true; syncRestart(); }
  // Renaming a page only changes its tab.
  const rebuild = keys.some(k => REBUILD.has(k) && (k !== "pages" || layouts(before) !== layouts(S())));
  if (rebuild) await O.reload();
  else O.applySettings();
}

const options = (list, value) => list.map(([v, label]) => `<option value="${v}"${String(v) === String(value) ? " selected" : ""}>${esc(label)}</option>`).join("");
const sw = (id, key, checked) => `<input id="${id}" type="checkbox" class="switch" data-set="${key}"${checked ? " checked" : ""}>`;

function render() {
  const s = S();
  const { monitors, settingsPath, info } = O.state;
  const monitorOpts = [["auto", "Automatic"], ...monitors.map(m => [m.index, `Display ${m.index} · ${m.width}×${m.height}${m.primary ? " (main)" : ""}`])];
  // A value set by hand in the settings file still shows as itself.
  const rotateOpts = ROTATE.some(([v]) => v === s.rotate_seconds) ? ROTATE : [...ROTATE, [s.rotate_seconds, `Every ${s.rotate_seconds} seconds`]];
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

      <section data-sk="pages tabs dashboards">
        <h3>Layout</h3>
        <div class="stack" data-pages data-s="pages tabs dashboards name rename order move delete remove">
          <p class="note">Each page is a dashboard of its own. Tabs in the header switch between them, and so do <kbd>1</kbd>–<kbd>9</kbd> and <kbd>PgUp</kbd> <kbd>PgDn</kbd>.</p>
          <div class="plist" id="s-pages"></div></div>
        <div class="stack" data-s="widgets tiles rows order arrange move add remove hide show reset drag pages tabs">
          <p class="note"><span class="rows-title" data-rows-title></span>Drag widgets to reorder them or move them between rows. <b>Edit layout</b> does that on the dashboard itself, and also resizes tiles and rows.</p>
          <div class="rows" id="s-rows"></div>
          <div class="actions"><button class="btn" type="button" data-edit-layout title="Move and resize tiles right on the dashboard (E)">Edit layout</button>
            <button class="btn" type="button" data-add-row>Add row</button>
            <button class="btn" type="button" data-add-page>Add page</button>
            <button class="btn link" type="button" data-reset-layout><span class="say">Reset to default</span><span class="ask">Reset all pages?</span></button></div></div>
        <div class="field" data-rotate data-s="rotate cycle pages switch automatically slideshow carousel next"><label for="s-rotate">Cycle pages<span class="hint">Moves on to the next page while nobody is using the dashboard</span></label>
          <select id="s-rotate" data-set="rotate_seconds" data-num>${options(rotateOpts, s.rotate_seconds)}</select></div>
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

      ${O.onlineSettings.html(s)}

      <section>
        <h3>About</h3>
        <p class="note" data-s="version license">Oriel ${esc(info.version)} · MIT License</p>
        <p class="note" data-s="file json location folder appdata">Settings are saved to <code>${esc(settingsPath)}</code></p>
        <p class="note" data-s="keyboard shortcuts keys hotkeys"><kbd>,</kbd> settings &nbsp;·&nbsp; <kbd>/</kbd> search &nbsp;·&nbsp; <kbd>Esc</kbd> close &nbsp;·&nbsp; <kbd>F11</kbd> full screen &nbsp;·&nbsp; <kbd>1</kbd>–<kbd>9</kbd> pages</p>
      </section>
      <p class="none" data-none hidden></p>
    </div>`;
  syncTheme();
  syncRestart();
  renderRows();
  O.onlineSettings.fill();
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
    const items = [...sec.querySelectorAll("[data-s]")].filter(it => !it.hidden); // e.g. page controls with one page
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
// Pages are only listed once there are two; until then the editor looks as it did before pages.
let editing = 0; // the page being edited, which the dashboard shows too
let armTimer = 0;

const ICON = {
  up: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 15 6-6 6 6"/></svg>`,
  down: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>`,
  x: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7 7 17"/></svg>`,
};

function renderRows() {
  const list = $("#s-pages");
  if (!list) return;
  const pages = S().pages;
  editing = Math.max(0, Math.min(pages.length - 1, editing));
  const many = pages.length > 1;
  drawer().querySelector("[data-pages]").hidden = !many;
  drawer().querySelector("[data-rotate]").hidden = !many;
  list.replaceChildren(...(many ? pages.map(pageRow) : []));
  const add = drawer().querySelector("[data-add-page]");
  add.disabled = pages.length >= MAX_PAGES;
  add.title = add.disabled ? `Up to ${MAX_PAGES} pages` : "";
  disarm();
  renderWidgets();
}

function pageRow(p, i, all) {
  return el(`
    <div class="prow${i === editing ? " on" : ""}" data-page-row="${i}">
      <span class="n">${i + 1}</span>
      <input type="text" class="pname" value="${esc(p.name)}" maxlength="${MAX_NAME}" spellcheck="false" autocomplete="off" aria-label="Name of page ${i + 1}">
      <button type="button" class="pbtn" data-move="-1" title="Move up" aria-label="Move page ${i + 1} up"${i ? "" : " disabled"}>${ICON.up}</button>
      <button type="button" class="pbtn" data-move="1" title="Move down" aria-label="Move page ${i + 1} down"${i < all.length - 1 ? "" : " disabled"}>${ICON.down}</button>
      <button type="button" class="pbtn del" data-del-page title="Delete page" aria-label="Delete page ${i + 1}">${ICON.x}<span class="ask">Delete?</span></button>
    </div>`);
}

/** The rows of the page being edited. */
function renderWidgets() {
  const box = $("#s-rows");
  const pages = S().pages;
  const { layout } = pages[editing];
  const title = drawer().querySelector("[data-rows-title]");
  title.hidden = pages.length < 2;
  title.textContent = `Widgets on ${pages[editing].name}`;
  // A widget can be on several pages, so only this page's widgets are left out of "Add…".
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

/** Save new pages, editing page `at` afterwards; the rebuilt dashboard opens on it too. */
function setPages(pages, at = editing) {
  editing = Math.max(0, Math.min(pages.length - 1, at));
  O.pages.remember(editing);
  const sent = JSON.stringify(pages);
  // Drawn now, and again only if the backend tidied something, so a name being typed isn't lost.
  save({ pages }).then(() => JSON.stringify(S().pages) !== sent && renderRows());
  renderRows();
}
const layoutOf = () => S().pages[editing].layout;
// The sizes saved with a layout only fit that layout, so the page goes back to the widgets' own sizes.
const setLayout = layout => setPages(S().pages.map((p, i) => (i === editing ? { ...p, layout, cols: [], rows: [] } : p)));

/** Edit page i, and show it on the dashboard behind the panel. */
function pick(i) {
  if (i === editing) return;
  editing = i;
  disarm();
  O.pages.show(i, true);
  $("#s-pages").querySelectorAll("[data-page-row]").forEach(r => r.classList.toggle("on", Number(r.dataset.pageRow) === i));
  renderWidgets();
}

function addPage() {
  const pages = S().pages;
  if (pages.length >= MAX_PAGES) return;
  const names = new Set(pages.map(p => p.name));
  let k = pages.length + 1;
  while (names.has(`Page ${k}`)) k++;
  // One empty row, so there's somewhere to add widgets.
  setPages([...pages, { name: `Page ${k}`, layout: [[]] }], pages.length);
  const input = $(`#s-pages [data-page-row="${pages.length}"] .pname`);
  input?.focus();
  input?.select(); // type a name straight away
}

function movePage(i, d) {
  const pages = [...S().pages];
  const j = i + d;
  if (j < 0 || j >= pages.length) return;
  [pages[i], pages[j]] = [pages[j], pages[i]];
  setPages(pages, editing === i ? j : editing === j ? i : editing);
  // Keep the keyboard on the same button, now one row along.
  const row = $(`#s-pages [data-page-row="${j}"]`);
  const btn = row?.querySelector(`[data-move="${d}"]`);
  (btn?.disabled ? row.querySelector(`[data-move="${-d}"]`) : btn)?.focus();
}

function deletePage(i, btn) {
  const pages = S().pages;
  if (pages.length < 2) return;
  if (pages[i].layout.some(r => r.length) && !armed(btn)) return arm(btn, `Click again to delete page ${i + 1} and its widgets`);
  setPages(pages.filter((_, k) => k !== i), editing > i ? editing - 1 : Math.min(editing, pages.length - 2));
}

function resetLayout(btn) {
  if (S().pages.length > 1 && !armed(btn)) return arm(btn, "Click again to replace every page with the default one");
  setPages([{ name: "Dashboard", layout: DEFAULT_LAYOUT.map(r => [...r]) }], 0);
}

// Deleting a page with widgets, or resetting several pages, asks for a second click within a few seconds.
const armed = btn => btn.classList.contains("armed");
function arm(btn, title) {
  disarm();
  btn.dataset.was = JSON.stringify([btn.title, btn.getAttribute("aria-label")]);
  btn.classList.add("armed");
  btn.title = title;
  btn.setAttribute("aria-label", title);
  armTimer = setTimeout(disarm, 4000);
}
function disarm() {
  clearTimeout(armTimer);
  for (const b of drawer().querySelectorAll(".armed")) {
    const [title, label] = JSON.parse(b.dataset.was);
    b.classList.remove("armed");
    b.title = title;
    if (label == null) b.removeAttribute("aria-label");
    else b.setAttribute("aria-label", label);
  }
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
    const layout = layoutOf().map(ids => ids.filter(id => id !== dragging));
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
      return setLayout(layoutOf().map(ids => ids.filter(x => x !== id)));
    }
    const prow = t.closest("[data-page-row]");
    if (prow) {
      const i = Number(prow.dataset.pageRow);
      const move = t.closest("[data-move]");
      if (move) return movePage(i, Number(move.dataset.move));
      const del = t.closest("[data-del-page]");
      if (del) return deletePage(i, del);
      return pick(i);
    }
    if (t.closest("[data-edit-layout]")) return O.editor.start();
    if (t.closest("[data-add-page]")) return addPage();
    if (t.closest("[data-add-row]")) return setLayout([...layoutOf(), []]);
    const reset = t.closest("[data-reset-layout]");
    if (reset) return resetLayout(reset);
    if (t.closest("[data-restart]")) invoke("restart");
  });
  // Clicking into a page's name picks that page as well.
  d.addEventListener("focusin", e => {
    if (e.target.matches(".pname")) pick(Number(e.target.closest("[data-page-row]").dataset.pageRow));
  });
  d.addEventListener("change", async e => {
    const t = e.target;
    if (t.matches("select.add") && t.value) {
      const r = Number(t.closest(".lrow").dataset.row);
      return setLayout(layoutOf().map((ids, i) => (i === r ? [...ids, t.value] : ids)));
    }
    if (t.matches(".pname")) {
      // Saved when the name is done (Enter or leaving the field), and only the tabs change.
      const i = Number(t.closest("[data-page-row]").dataset.pageRow);
      await save({ pages: S().pages.map((p, k) => (k === i ? { ...p, name: t.value } : p)) });
      if (document.activeElement !== t) t.value = S().pages[i]?.name ?? t.value; // as the backend tidied it
      return renderWidgets();
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
  // Esc in a non-empty search clears it; the next Esc closes the panel. Enter finishes a page name.
  d.addEventListener("keydown", e => {
    if (e.key === "Enter" && e.target.matches(".pname")) return e.target.blur();
    const q = searchInput();
    if (e.key !== "Escape" || e.target !== q || !q.value) return;
    e.preventDefault();
    e.stopPropagation();
    q.value = "";
    search("");
  });
  $("#scrim").addEventListener("click", close);
  O.onlineSettings.wire({ save, S, drawer });
}

// ---- open / close --------------------------------------------------------------------
const isOpen = () => drawer().classList.contains("open");
function open() {
  editing = O.pages.current();
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
  if (drawer().contains(document.activeElement)) document.activeElement.blur(); // saves a page name being typed
  drawer().classList.remove("open");
  drawer().inert = true;
  $("#scrim").classList.remove("open");
  O.hideTip();
  // Empty rows are kept while editing so there's somewhere to drop; tidy them away now.
  const pages = S().pages;
  if (pages.some(p => p.layout.some(r => !r.length))) {
    O.pages.remember(editing);
    save({ pages: pages.map(p => (p.layout.every(r => r.length) ? p : { ...p, layout: p.layout.filter(r => r.length), cols: [], rows: [] })) });
  }
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
