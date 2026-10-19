"use strict";
// Oriel core: the widget registry, formatting, charts and the tooltip.
// Plain scripts with no build step; each file adds to the global `Oriel`.

const Oriel = (window.Oriel = {
  widgets: new Map(), // id -> definition, in registration order
  charts: [],         // every chart on every page, fed one sample per tick
  state: { info: null, settings: null, interval: 1, monitors: [], autostart: false, settingsPath: "" },
  tokens: {},         // CSS colors, read once per theme change rather than per frame
  fontPx: 16,
});

{
const O = Oriel;

// ---- widgets -------------------------------------------------------------------
/**
 * Register a widget.
 *   name, description   shown in the settings layout editor
 *   color               token name for its dot in the editor ("cpu", "gpu", ...)
 *   size: {w, h}        width within its row, and how tall its row should be (relative)
 *   instances(info)     optional: one tile per item, e.g. per graphics card
 *   create(ctx)         returns {root, update(sample)}. ctx: {info, inst, chart, go}
 */
O.register = (id, def) => O.widgets.set(id, { id, color: "muted", ...def, size: { w: 1, h: 1, ...def.size } });

/** Ask Windows to open something (see `run_action` in main.rs for the allowed list). */
O.action = name => window.__TAURI__.core.invoke("run_action", { name }).catch(e => console.warn(e));

// ---- DOM -----------------------------------------------------------------------
O.$ = (sel, el = document) => el.querySelector(sel);
O.esc = s => String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
O.el = html => {
  const t = document.createElement("template");
  t.innerHTML = html.trim();
  return t.content.firstElementChild;
};
/** Collect children marked data-k="name" into {root, name: node, ...}. */
O.refs = root => {
  const r = { root };
  root.querySelectorAll("[data-k]").forEach(n => (r[n.dataset.k] = n));
  return r;
};
// Writes that skip the DOM when nothing changed: most values hold still between ticks.
O.setHTML = (node, s) => { if (node._h !== s) { node._h = s; node.innerHTML = s; } };
O.setText = (node, s) => { if (node._t !== s) { node._t = s; node.textContent = s; } };
O.meter = (node, pct, lvl = "") => {
  const w = Math.max(0, Math.min(100, pct || 0)).toFixed(1);
  const key = w + lvl;
  if (node._k === key) return;
  node._k = key;
  node.firstElementChild.style.width = `${w}%`;
  node.className = lvl ? `meter ${lvl}` : "meter";
};

const GO_ICON = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 17 17 7M8 7h9v9"/></svg>`;
/** A small "open in Windows" button for a tile header. */
O.go = (action, title) => `<button class="go" type="button" data-action="${O.esc(action)}" title="${O.esc(title)}" aria-label="${O.esc(title)}">${GO_ICON}</button>`;

const FIND_ICON = `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/></svg>`;
/** A compact search field (the process list and the settings panel use it). */
O.findBox = (label, placeholder) => `<label class="find">${FIND_ICON}<input type="search" placeholder="${O.esc(placeholder)}" aria-label="${O.esc(label)}" spellcheck="false" autocomplete="off"></label>`;

// ---- theme tokens --------------------------------------------------------------
const TOKENS = ["page", "surface", "surface-hi", "ink", "ink-2", "muted", "grid", "baseline", "cpu", "gpu", "mem", "s1", "s2", "warn", "crit"];
O.readTokens = () => {
  const cs = getComputedStyle(document.documentElement);
  for (const k of TOKENS) O.tokens[k] = cs.getPropertyValue(`--${k}`).trim();
  O.fontPx = parseFloat(cs.fontSize) || 16;
};

// ---- formatting ----------------------------------------------------------------
const UNITS = ["B", "KB", "MB", "GB", "TB"];
function splitBytes(n) {
  if (n == null || Number.isNaN(n)) return ["–", ""];
  let i = 0;
  while (n >= 1000 && i < UNITS.length - 1) { n /= 1024; i++; }
  const s = i === 0 ? Math.round(n).toString() : n.toFixed(n < 100 ? 1 : 0);
  return [s.replace(/\.0$/, ""), UNITS[i]];
}
const bytes = n => splitBytes(n).join(" ");
O.fmt = {
  splitBytes,
  bytes,
  rate: n => bytes(n) + "/s",
  rateHTML: n => { const [v, u] = splitBytes(n); return `${v}<small>${u}/s</small>`; },
  pctHTML: v => (v == null ? "–" : `${Math.round(v)}<small>%</small>`),
  gb: n => (n / 1024 ** 3).toFixed(1),
  dur(s) {
    const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
    return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`;
  },
  ago(s) {
    s = Math.round(s);
    if (s < 1) return "now";
    if (s < 60) return `${s}s ago`;
    return `${Math.floor(s / 60)}m ${s % 60}s ago`;
  },
  /** Round a byte rate up to 1/2/5 × 10^k in its display unit, so the axis reads "5 MB/s", not "4.77 MB/s". */
  niceCeil(v) {
    let unit = 1;
    while (v / unit >= 1000) unit *= 1024;
    const x = v / unit;
    const p = 10 ** Math.floor(Math.log10(x || 1));
    const m = [1, 2, 5, 10].find(m => m * p >= x) || 10;
    return m * p * unit;
  },
};
O.facts = parts => parts.filter(Boolean).map(x => `<span>${x}</span>`).join("");

// ---- thresholds ----------------------------------------------------------------
O.limits = () => O.state.settings.limits;
O.level = (v, [warn, crit]) => (v == null ? "" : v >= crit ? "crit" : v >= warn ? "warn" : "");
O.tempHTML = (t, limits, source) => {
  if (t == null) return "";
  const lvl = O.level(t, limits);
  const chip = lvl ? ` <span class="chip ${lvl}">${lvl === "crit" ? "Hot" : "Warm"}</span>` : "";
  const src = source === "acpi"
    ? `<span class="src" title="ACPI thermal zone, not the CPU's own sensor. Run LibreHardwareMonitor for that (see Settings).">acpi</span>`
    : "";
  return `${Math.round(t)} °C${src}${chip}`;
};

// ---- tooltip -------------------------------------------------------------------
const tip = () => O.$("#tip");
O.showTip = (x, y, html) => {
  const t = tip();
  t.innerHTML = html;
  t.hidden = false;
  const r = t.getBoundingClientRect();
  const left = x + 14 + r.width > innerWidth ? x - 14 - r.width : x + 14;
  t.style.left = `${Math.max(4, left)}px`;
  t.style.top = `${Math.min(innerHeight - r.height - 4, Math.max(4, y - r.height / 2))}px`;
};
O.hideTip = () => (tip().hidden = true);

// ---- canvases ------------------------------------------------------------------
class CanvasBox {
  constructor(el) {
    this.el = el;
    this.canvas = document.createElement("canvas");
    el.appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d");
    this.hoverX = null;
    new ResizeObserver(() => this.resize()).observe(el);
    this.canvas.addEventListener("pointermove", e => { this.hoverX = e.offsetX; this.hoverY = e.clientY; this.draw(); });
    this.canvas.addEventListener("pointerleave", () => { this.hoverX = null; O.hideTip(); this.draw(); });
  }
  resize() {
    const dpr = devicePixelRatio || 1;
    this.w = this.el.clientWidth;
    this.h = this.el.clientHeight;
    this.canvas.width = Math.round(this.w * dpr);
    this.canvas.height = Math.round(this.h * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.draw();
  }
}

/** Time series. series: [{label, color: token name, get: sample => number}]; max: number | "auto". */
O.Chart = class Chart extends CanvasBox {
  constructor(el, { series, max = 100, floor = 1, fmt = v => `${Math.round(v)}%` }) {
    super(el);
    this.series = series;
    this.max = max;
    this.floor = floor;
    this.fmt = fmt;
    this.data = series.map(() => []);
    this.cap = 2;
    this.off = false; // on a page that isn't showing: keep taking samples, skip the drawing
  }
  push(sample) {
    this.series.forEach((s, i) => {
      const a = this.data[i];
      a.push(s.get(sample) ?? 0);
      if (a.length > this.cap) a.splice(0, a.length - this.cap);
    });
  }
  draw() {
    const { ctx, w, h } = this;
    if (!w || !h || this.off || document.hidden) return;
    const T = O.tokens;
    ctx.clearRect(0, 0, w, h);
    const fs = O.fontPx;
    const top = Math.ceil(fs * 0.2), bottom = h - 1;
    const step = w / (this.cap - 1);
    let max = this.max;
    if (max === "auto") {
      let m = this.floor;
      for (const a of this.data) for (const v of a) if (v > m) m = v;
      max = O.fmt.niceCeil(m);
    }
    const y = v => bottom - (Math.min(v, max) / max) * (bottom - top);

    // recessive grid: 50% and 100% hairlines, solid baseline
    ctx.lineWidth = 1;
    ctx.strokeStyle = T.grid;
    for (const f of [0.5, 1]) {
      const yy = Math.round(y(max * f)) + 0.5;
      ctx.beginPath(); ctx.moveTo(0, yy); ctx.lineTo(w, yy); ctx.stroke();
    }
    ctx.strokeStyle = T.baseline;
    ctx.beginPath(); ctx.moveTo(0, bottom + 0.5); ctx.lineTo(w, bottom + 0.5); ctx.stroke();
    ctx.fillStyle = T.muted;
    ctx.font = `${fs * 0.72}px "Segoe UI", system-ui, sans-serif`;
    ctx.textBaseline = "top";
    ctx.fillText(this.fmt(max), 2, y(max) + 3);

    const pts = [];
    this.series.forEach((s, si) => {
      const a = this.data[si];
      if (a.length < 2) return;
      const color = T[s.color];
      const x0 = w - (a.length - 1) * step;
      const trace = () => {
        ctx.beginPath();
        a.forEach((v, i) => (i ? ctx.lineTo(x0 + i * step, y(v)) : ctx.moveTo(x0, y(v))));
      };
      if (this.series.length === 1) { // a light wash under a single series
        trace();
        ctx.lineTo(w, bottom); ctx.lineTo(x0, bottom); ctx.closePath();
        ctx.globalAlpha = 0.12; ctx.fillStyle = color; ctx.fill(); ctx.globalAlpha = 1;
      }
      trace();
      ctx.lineWidth = 2; ctx.lineJoin = "round"; ctx.lineCap = "round";
      ctx.strokeStyle = color; ctx.stroke();
      pts.push({ s, a, color });
    });

    // hover crosshair, else a dot on the latest value
    let back = 0;
    if (this.hoverX != null && pts.length) {
      back = Math.max(0, Math.min(pts[0].a.length - 1, Math.round((w - this.hoverX) / step)));
      const xx = Math.round(w - back * step) + 0.5;
      ctx.strokeStyle = T.muted; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(xx, top); ctx.lineTo(xx, bottom); ctx.stroke();
      const rows = pts.map(p => {
        const v = p.a[p.a.length - 1 - back];
        return `<div class="r"><span class="sw" style="--c:${p.color}"></span>${O.esc(p.s.label)}<b>${O.esc(this.fmt(v))}</b></div>`;
      }).join("");
      const box = this.canvas.getBoundingClientRect();
      O.showTip(box.left + xx, this.hoverY, `<div class="ago">${O.fmt.ago(back * O.state.interval)}</div>${rows}`);
    }
    for (const p of pts) {
      const i = p.a.length - 1 - back;
      if (i < 0) continue;
      const xx = w - back * step, yy = y(p.a[i]);
      ctx.beginPath(); ctx.arc(Math.min(xx, w - 5), yy, 4, 0, Math.PI * 2);
      ctx.fillStyle = p.color; ctx.fill();
      ctx.lineWidth = 2; ctx.strokeStyle = T.surface; ctx.stroke();
    }
  }
};

/** One bar per value (0–100), e.g. per CPU thread. */
O.Bars = class Bars extends CanvasBox {
  constructor(el, color, label = i => `Thread ${i}`) { super(el); this.color = color; this.label = label; this.values = []; }
  set(values) { this.values = values; this.draw(); }
  draw() {
    const { ctx, w, h, values } = this;
    if (!w || !h || !values.length || document.hidden) return;
    ctx.clearRect(0, 0, w, h);
    const n = values.length;
    let gap = 2;
    let bw = Math.min(24, (w - gap * (n - 1)) / n);
    if (bw < 3) { gap = 1; bw = (w - gap * (n - 1)) / n; }
    const x0 = (w - (n * bw + (n - 1) * gap)) / 2;
    const color = O.tokens[this.color];
    const r = Math.min(4, bw / 2);
    const hovered = this.hoverX == null ? -1 : Math.floor((this.hoverX - x0) / (bw + gap));
    ctx.fillStyle = color;
    values.forEach((v, i) => {
      const x = x0 + i * (bw + gap);
      ctx.globalAlpha = i === hovered ? 0.3 : 0.16; // track
      ctx.beginPath(); ctx.roundRect(x, 0, bw, h, [r, r, 0, 0]); ctx.fill();
      ctx.globalAlpha = 1;
      const fh = (Math.min(v, 100) / 100) * h;
      if (fh > 0.5) {
        ctx.beginPath(); ctx.roundRect(x, h - fh, bw, fh, [Math.min(r, fh), Math.min(r, fh), 0, 0]); ctx.fill();
      }
    });
    if (hovered >= 0 && hovered < n) {
      const box = this.canvas.getBoundingClientRect();
      O.showTip(box.left + x0 + hovered * (bw + gap) + bw / 2, this.hoverY,
        `<div class="r"><span class="sw" style="--c:${color}"></span>${O.esc(this.label(hovered))}<b>${Math.round(values[hovered])}%</b></div>`);
    } else if (this.hoverX != null) O.hideTip();
  }
};

O.drawCharts = () => {
  if (!document.hidden) for (const c of O.charts) if (!c.off) c.draw();
};
}
