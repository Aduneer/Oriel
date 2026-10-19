"use strict";
// Edit layout: change the page on screen where it is. Drag a tile to move it, drag the edges between
// tiles or rows to resize them, remove tiles, add widgets. Changes save as they're made.
// Sizes are weights: how the page's grid shares out the room (see O.fr), saved as `cols` and `rows`.
{
const O = Oriel;
const { $, el, esc } = O;
const { invoke } = window.__TAURI__.core;

/** Weights to CSS grid tracks, scaled so they add up to the number of tracks: fr tracks that add up to less than 1 wouldn't fill the page. */
O.fr = ws => {
  if (!ws.length) return "1fr";
  const k = ws.length / ws.reduce((a, b) => a + b, 0);
  return ws.map(w => `${(w * k).toFixed(4)}fr`).join(" ");
};

const MIN = 0.2, MAX = 8; // as in settings.rs
const clamp = w => Math.max(MIN, Math.min(MAX, w));
const narrow = matchMedia("(max-aspect-ratio: 1/1), (max-width: 720px)"); // tiles wrap or stack: no edges to drag

let active = false;
let cur = 0;          // the page being edited
let m = null;         // {layout, cols, rows} of that page, with every size filled in
let root = null;      // the page's element
let layer = null;     // dividers and the drop marker, over the page
let bar = null;       // Done, Add widget...
let busy = false;     // a change is being saved
let focusAt = null;   // {r, k}: the tile to put the keyboard back on after a rebuild
let resizer = null;
let rowEls = [];      // the rows on screen
const covers = new Map(); // cover element -> {r, k}

// ---- the page's sizes ------------------------------------------------------------------
function readModel() {
  const p = O.state.settings.pages[cur];
  const layout = p.layout.map(r => [...r]);
  const wOf = id => O.widgets.get(id)?.size.w ?? 1;
  const hOf = id => O.widgets.get(id)?.size.h ?? 1;
  const colsOk = p.cols?.length === layout.length && p.cols.every((c, i) => c.length === layout[i].length);
  const rowsOk = p.rows?.length === layout.length;
  return {
    layout,
    cols: layout.map((ids, r) => (colsOk ? [...p.cols[r]] : ids.map(wOf))),
    rows: layout.map((ids, r) => (rowsOk ? p.rows[r] : Math.max(0, ...ids.map(hOf)) || 1)),
  };
}

async function commit(rebuild) {
  busy = true;
  try {
    const s = O.state.settings;
    const pages = s.pages.map((p, i) => (i === cur ? { ...p, layout: m.layout, cols: m.cols, rows: m.rows } : p));
    O.state.settings = { ...s, pages };
    try {
      O.state.settings = await invoke("save_settings", { settings: O.state.settings });
    } catch (e) {
      console.warn("couldn't save settings", e);
    }
    O.pages.remember(cur);
    if (rebuild) await O.reload(); // refresh() runs at the end of it
  } finally {
    busy = false;
  }
}

/** The tiles of a row, grouped by the layout entry they came from (a widget like GPU can make several). */
function entriesOf(row) {
  const ids = m.layout[Number(row.dataset.r)];
  const out = [];
  let k = 0;
  for (const tile of row.children) {
    while (k < ids.length && ids[k] !== tile.dataset.widget) k++;
    if (k >= ids.length) break;
    (out.find(e => e.k === k) || out[out.push({ k, tiles: [] }) - 1]).tiles.push(tile);
  }
  return out;
}

/** Show a size change straight away, before it's saved. */
function applyRow(row) {
  const r = Number(row.dataset.r);
  const ws = [];
  for (const e of entriesOf(row)) for (const _ of e.tiles) ws.push(m.cols[r][e.k]);
  row.style.setProperty("--cols", O.fr(ws));
}
function applyRows() {
  const hs = rowEls.map(row => m.rows[Number(row.dataset.r)]);
  root.style.setProperty("--rows", O.fr(hs));
  root.style.setProperty("--rows-p", O.fr(rowEls.map((row, i) => hs[i] * Math.ceil(row.children.length / 2))));
}

// ---- changes to the layout ------------------------------------------------------------
/**
 * Move entry `from` {r, k} to `to`: {r, at} (a place in a row) or {newRow} (before that row of the
 * layout, or at the end). Returns where it ended up, or null when nothing moved.
 */
function move(from, to) {
  const { layout, cols, rows } = m;
  const srcRow = layout[from.r];
  const tgtRow = to.r != null ? layout[to.r] : null;
  const before = to.newRow != null ? layout[to.newRow] : null;
  let at = to.at;
  if (tgtRow === srcRow) {
    if (at === from.k || at === from.k + 1) return null;
    if (at > from.k) at--;
  }
  const id = srcRow[from.k];
  const w = cols[from.r][from.k];
  const h = rows[from.r];
  srcRow.splice(from.k, 1);
  cols[from.r].splice(from.k, 1);
  let placed;
  if (tgtRow) {
    const t = layout.indexOf(tgtRow);
    tgtRow.splice(at, 0, id);
    cols[t].splice(at, 0, w);
    placed = tgtRow;
  } else {
    const t = before ? layout.indexOf(before) : layout.length;
    placed = [id];
    layout.splice(t, 0, placed);
    cols.splice(t, 0, [w]);
    rows.splice(t, 0, h);
  }
  for (let i = layout.length - 1; i >= 0; i--) {
    if (layout[i].length) continue;
    layout.splice(i, 1);
    cols.splice(i, 1);
    rows.splice(i, 1);
  }
  return { r: layout.indexOf(placed), k: tgtRow ? at : 0 };
}

function remove({ r, k }) {
  const { layout, cols, rows } = m;
  layout[r].splice(k, 1);
  cols[r].splice(k, 1);
  if (!layout[r].length) {
    layout.splice(r, 1);
    cols.splice(r, 1);
    rows.splice(r, 1);
  }
  focusAt = null;
  commit(true);
}

function add(id) {
  const def = O.widgets.get(id);
  if (!def || busy) return;
  const { layout, cols, rows } = m;
  const last = layout.length - 1;
  if (last >= 0 && layout[last].length < 3) {
    layout[last].push(id);
    cols[last].push(def.size.w);
    focusAt = { r: last, k: layout[last].length - 1 };
  } else {
    layout.push([id]);
    cols.push([def.size.w]);
    rows.push(def.size.h);
    focusAt = { r: layout.length - 1, k: 0 };
  }
  commit(true);
}

function resetSizes() {
  if (busy) return;
  m.cols = []; // none saved means each widget's own size
  m.rows = [];
  commit(true);
}

// ---- the overlay ------------------------------------------------------------------------
const GRIP = `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="9" cy="6" r="1.3"/><circle cx="15" cy="6" r="1.3"/><circle cx="9" cy="12" r="1.3"/><circle cx="15" cy="12" r="1.3"/><circle cx="9" cy="18" r="1.3"/><circle cx="15" cy="18" r="1.3"/></svg>`;
const X = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7 7 17"/></svg>`;

function teardownOverlay() {
  resizer?.disconnect();
  resizer = null;
  layer?.remove();
  layer = null;
  for (const [cover] of covers) {
    const tile = cover.parentElement;
    cover.remove();
    tile.classList.remove("ed");
    for (const c of tile.children) c.inert = false;
  }
  covers.clear();
}

function setUpOverlay() {
  teardownOverlay();
  root = document.querySelectorAll("#grid > .page")[cur];
  if (!root) return;
  m = readModel();
  rowEls = [...root.querySelectorAll(":scope > .row")];
  for (const row of rowEls) {
    const r = Number(row.dataset.r);
    for (const { k, tiles } of entriesOf(row)) {
      const name = O.widgets.get(m.layout[r][k])?.name ?? m.layout[r][k];
      for (const tile of tiles) {
        for (const c of tile.children) c.inert = true; // the tile's own buttons wait until editing is done
        tile.classList.add("ed");
        const cover = el(`<div class="ed-cover" tabindex="0" role="group" aria-label="${esc(name)}. Arrow keys move it, Alt and arrow keys resize it, Delete removes it.">
          <span class="ed-name">${GRIP}${esc(name)}</span>
          <button type="button" class="ed-x" tabindex="-1" title="Remove ${esc(name)}" aria-label="Remove ${esc(name)}">${X}</button></div>`);
        tile.appendChild(cover);
        covers.set(cover, { r, k });
      }
    }
  }
  layer = el(`<div class="ed-layer" aria-hidden="true"><i class="ed-mark" hidden></i></div>`);
  root.appendChild(layer);
  placeDividers();
  resizer = new ResizeObserver(placeDividers);
  resizer.observe(root);
  if (focusAt) {
    const target = [...covers].find(([, c]) => c.r === focusAt.r && c.k === focusAt.k);
    (target?.[0] ?? covers.keys().next().value)?.focus({ preventScroll: true });
    focusAt = null;
  }
}

/** A grab handle on every edge between two tiles in a row, and between two rows. */
function placeDividers() {
  if (!layer) return;
  layer.querySelectorAll(".ed-div").forEach(d => d.remove());
  const box = root.getBoundingClientRect();
  const gap = O.fontPx * 0.75;
  const add = (cls, x, y, w, h, data) => {
    const d = el(`<i class="ed-div ${cls}"></i>`);
    Object.assign(d.style, { left: `${x}px`, top: `${y}px`, width: `${w}px`, height: `${h}px` });
    Object.assign(d.dataset, data);
    layer.appendChild(d);
  };
  rowEls.forEach((row, i) => {
    const rb = row.getBoundingClientRect();
    const es = entriesOf(row);
    for (let j = 0; j + 1 < es.length; j++) {
      const a = es[j].tiles.at(-1).getBoundingClientRect(), b = es[j + 1].tiles[0].getBoundingClientRect();
      const mid = (a.right + b.left) / 2 - box.left;
      add("v", mid - gap, rb.top - box.top, gap * 2, rb.height, { r: row.dataset.r, k: es[j].k, k2: es[j + 1].k });
    }
    const next = rowEls[i + 1];
    if (next) {
      const nb = next.getBoundingClientRect();
      const mid = (rb.bottom + nb.top) / 2 - box.top;
      add("h", 0, mid - gap, box.width, gap * 2, { i });
    }
  });
}

// ---- dragging ---------------------------------------------------------------------------
/** Where a dragged tile would land with the pointer at (x, y). */
function dropAt(x, y) {
  const box = root.getBoundingClientRect();
  if (x < box.left || x > box.right) return null;
  const pad = O.fontPx * 0.9;
  const rects = rowEls.map(row => row.getBoundingClientRect());
  // Between rows, and above the first and below the last: a row of its own.
  for (let i = 0; i <= rowEls.length; i++) {
    const edge = i === 0 ? rects[0]?.top : i === rowEls.length ? rects[i - 1].bottom : (rects[i - 1].bottom + rects[i].top) / 2;
    if (edge == null || Math.abs(y - edge) > pad) continue;
    const before = rowEls[i];
    return { to: { newRow: before ? Number(before.dataset.r) : m.layout.length }, mark: { x: 0, y: edge - box.top - 2, w: box.width, h: 4 } };
  }
  // Otherwise the tile nearest the pointer: before it or after it.
  let best = null;
  for (const row of rowEls) {
    const r = Number(row.dataset.r);
    for (const { k, tiles } of entriesOf(row)) {
      for (const tile of tiles) {
        const b = tile.getBoundingClientRect();
        const d = Math.hypot(Math.max(b.left - x, 0, x - b.right), Math.max(b.top - y, 0, y - b.bottom));
        if (!best || d < best.d) best = { d, r, k, tile, tiles, b };
      }
    }
  }
  if (!best) return { to: { newRow: 0 }, mark: null };
  const stacked = matchMedia("(max-width: 720px)").matches;
  const after = stacked ? y > (best.b.top + best.b.bottom) / 2 : x > (best.b.left + best.b.right) / 2;
  const edge = after ? best.tiles.at(-1).getBoundingClientRect() : best.tiles[0].getBoundingClientRect();
  const mark = stacked
    ? { x: edge.left - box.left, y: (after ? edge.bottom : edge.top) - box.top - 2, w: edge.width, h: 4 }
    : { x: (after ? edge.right : edge.left) - box.left - 2, y: edge.top - box.top, w: 4, h: edge.height };
  return { to: { r: best.r, at: best.k + (after ? 1 : 0) }, mark };
}

function showMark(mark) {
  const i = layer.querySelector(".ed-mark");
  i.hidden = !mark;
  if (mark) Object.assign(i.style, { left: `${mark.x}px`, top: `${mark.y}px`, width: `${mark.w}px`, height: `${mark.h}px` });
}

let drag = null; // {cover, from, x, y, on, drop}
function endDrag() {
  if (!drag) return;
  drag.cover.classList.remove("dragging");
  drag.cover.parentElement.classList.remove("lifted");
  root.classList.remove("moving");
  showMark(null);
  drag = null;
}

document.addEventListener("pointerdown", e => {
  if (!active || e.button !== 0 || busy) return;
  const x = e.target.closest(".ed-x");
  const cover = e.target.closest(".ed-cover");
  if (x && cover) {
    e.preventDefault();
    return remove(covers.get(cover));
  }
  if (cover) {
    drag = { cover, from: covers.get(cover), x: e.clientX, y: e.clientY, on: false, drop: null, id: e.pointerId };
    cover.setPointerCapture(e.pointerId);
    return;
  }
  const div = e.target.closest(".ed-div");
  if (div) startResize(div, e);
});
document.addEventListener("pointermove", e => {
  if (!active) return;
  if (grab) return doResize(e);
  if (!drag || e.pointerId !== drag.id) return;
  if (!drag.on) {
    if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6) return;
    drag.on = true;
    drag.cover.classList.add("dragging");
    drag.cover.parentElement.classList.add("lifted");
    root.classList.add("moving");
  }
  drag.drop = dropAt(e.clientX, e.clientY);
  showMark(drag.drop?.mark);
});
document.addEventListener("pointerup", e => {
  if (grab) return finishResize();
  if (!drag || e.pointerId !== drag.id) return;
  const { drop, from, on } = drag;
  endDrag();
  if (!on || !drop) return;
  const to = move(from, drop.to);
  if (to) {
    focusAt = to;
    commit(true);
  }
});
document.addEventListener("pointercancel", () => {
  endDrag();
  if (grab) finishResize();
});

// ---- resizing ---------------------------------------------------------------------------
let grab = null; // {div, row, r, k, k2, a, b, start, ga, gb, total, vertical}
function startResize(div, e) {
  e.preventDefault();
  div.setPointerCapture(e.pointerId);
  const vertical = div.classList.contains("h");
  if (vertical) {
    const i = Number(div.dataset.i);
    const [ra, rb] = [rowEls[i], rowEls[i + 1]];
    const [a, b] = [ra.getBoundingClientRect().height, rb.getBoundingClientRect().height];
    grab = { div, vertical, start: e.clientY, a, b, ia: Number(ra.dataset.r), ib: Number(rb.dataset.r) };
    grab.ga = m.rows[grab.ia];
    grab.gb = m.rows[grab.ib];
  } else {
    const row = rowEls.find(r => r.dataset.r === div.dataset.r);
    const es = entriesOf(row);
    const ea = es.find(x => x.k === Number(div.dataset.k)), eb = es.find(x => x.k === Number(div.dataset.k2));
    grab = {
      div, vertical, row, r: Number(div.dataset.r), ka: ea.k, kb: eb.k, start: e.clientX,
      a: ea.tiles[0].getBoundingClientRect().width, b: eb.tiles[0].getBoundingClientRect().width,
    };
    grab.ga = m.cols[grab.r][grab.ka];
    grab.gb = m.cols[grab.r][grab.kb];
  }
  div.classList.add("on");
}
function doResize(e) {
  const g = grab;
  const sum = g.ga + g.gb;
  const px = g.a + g.b;
  // Pixels per unit of weight is the same for both sides of the edge.
  const unit = px / sum;
  const lo = Math.max(MIN, O.fontPx * (g.vertical ? 6 : 12) / unit), hi = sum - lo;
  const d = ((g.vertical ? e.clientY : e.clientX) - g.start) / unit;
  const ga = Math.min(hi, Math.max(lo, g.ga + d));
  if (g.vertical) {
    m.rows[g.ia] = ga;
    m.rows[g.ib] = sum - ga;
    applyRows();
  } else {
    m.cols[g.r][g.ka] = ga;
    m.cols[g.r][g.kb] = sum - ga;
    applyRow(g.row);
  }
  placeDividers();
  // placeDividers rebuilt the handles, so the one being dragged is a new element.
  layer.querySelectorAll(".ed-div").forEach(n => {
    if (g.vertical ? n.dataset.i === g.div.dataset.i : n.dataset.r === g.div.dataset.r && n.dataset.k === g.div.dataset.k) n.classList.add("on");
  });
}
function finishResize() {
  grab = null;
  layer?.querySelectorAll(".ed-div.on").forEach(n => n.classList.remove("on"));
  m.cols = m.cols.map(r => r.map(clamp));
  m.rows = m.rows.map(clamp);
  commit(false);
}

// ---- keyboard ---------------------------------------------------------------------------
function keyOnCover(e, cover) {
  const at = covers.get(cover);
  if (!at || busy) return;
  const { layout } = m;
  const { r, k } = at;
  const key = e.key;
  if (key === "Delete" || key === "Backspace") {
    e.preventDefault();
    return remove(at);
  }
  if (!key.startsWith("Arrow")) return;
  e.preventDefault();
  if (e.altKey) {
    // Alt + arrows: wider or narrower, taller or shorter
    const f = key === "ArrowRight" || key === "ArrowDown" ? 1.1 : 1 / 1.1;
    if (key === "ArrowLeft" || key === "ArrowRight") m.cols[r][k] = clamp(m.cols[r][k] * f);
    else m.rows[r] = clamp(m.rows[r] * f);
    focusAt = at;
    const row = rowEls.find(x => Number(x.dataset.r) === r);
    applyRow(row);
    applyRows();
    placeDividers();
    return commit(false);
  }
  let to = null;
  if (key === "ArrowLeft") to = k > 0 ? { r, at: k - 1 } : r > 0 ? { r: r - 1, at: layout[r - 1].length } : null;
  else if (key === "ArrowRight") to = k < layout[r].length - 1 ? { r, at: k + 2 } : r < layout.length - 1 ? { r: r + 1, at: 0 } : null;
  else if (key === "ArrowUp") to = r > 0 ? { r: r - 1, at: Math.min(k, layout[r - 1].length) } : layout[r].length > 1 ? { newRow: 0 } : null;
  else to = r < layout.length - 1 ? { r: r + 1, at: Math.min(k, layout[r + 1].length) } : layout[r].length > 1 ? { newRow: layout.length } : null;
  const placed = to && move(at, to);
  if (placed) {
    focusAt = placed;
    commit(true);
  }
}

// Capture phase: while editing, these keys are for editing and nothing else.
addEventListener("keydown", e => {
  if (!active) return;
  if (e.key === "Escape") {
    e.stopImmediatePropagation();
    if (drag) endDrag();
    else stop();
    return;
  }
  const cover = e.target.closest?.(".ed-cover");
  if (cover) keyOnCover(e, cover);
  if (e.target.closest("select, button, input")) return;
  if (e.key === "e" || /^[1-9,/]$/.test(e.key) || e.key === "PageUp" || e.key === "PageDown") {
    e.stopImmediatePropagation();
    if (e.key === "e") stop();
  }
}, true);

// ---- start and stop --------------------------------------------------------------------------
function makeBar() {
  const free = () => [...O.widgets.values()].filter(w => !m.layout.flat().includes(w.id));
  bar = el(`<div class="ed-bar" role="toolbar" aria-label="Edit layout">
    <span class="ed-title"><b>Editing layout</b><span data-k="page"></span></span>
    <select data-k="add" aria-label="Add a widget"></select>
    <button type="button" class="btn" data-k="reset" title="Back to each widget's own size">Reset sizes</button>
    <button type="button" class="btn primary" data-k="done">Done</button>
    <span class="ed-hint">Drag a tile to move it · drag an edge to resize · <kbd>Esc</kbd> to finish</span></div>`);
  document.body.appendChild(bar);
  bar.querySelector('[data-k="done"]').addEventListener("click", stop);
  bar.querySelector('[data-k="reset"]').addEventListener("click", resetSizes);
  const select = bar.querySelector('[data-k="add"]');
  select.addEventListener("change", () => {
    const id = select.value;
    select.value = "";
    if (id) add(id);
  });
  bar.refresh = () => {
    const pages = O.state.settings.pages;
    bar.querySelector('[data-k="page"]').textContent = pages.length > 1 ? ` · ${pages[cur].name}` : "";
    const list = free();
    select.innerHTML = `<option value="">Add a widget…</option>${list.map(w => `<option value="${esc(w.id)}">${esc(w.name)}</option>`).join("")}`;
    select.hidden = !list.length;
  };
}

function start() {
  if (active || !O.state.settings) return;
  O.settingsPanel.close();
  active = true;
  cur = O.pages.current();
  document.body.classList.add("editing");
  m = readModel();
  makeBar();
  refresh();
  (covers.keys().next().value)?.focus({ preventScroll: true });
}

function stop() {
  if (!active) return;
  endDrag();
  grab = null;
  active = false;
  teardownOverlay();
  bar?.remove();
  bar = null;
  document.body.classList.remove("editing");
}

/** The dashboard was rebuilt (or this is the start): put the controls back over it. */
function refresh() {
  if (!active) return;
  cur = O.pages.current();
  setUpOverlay();
  bar?.refresh();
}

// Going elsewhere ends editing: another page, the settings.
$("#pages").addEventListener("click", stop, true);
$("#gearBtn").addEventListener("click", stop, true);
addEventListener("resize", () => active && placeDividers());
narrow.addEventListener("change", () => active && placeDividers());

O.editor = {
  get active() { return active; },
  start,
  stop,
  toggle: () => (active ? stop() : start()),
  refresh,
};
}
