"use strict";
// Dials for the numbers you glance at most: load, temperatures and memory. The warn and critical
// limits from Settings → Alerts show as bands on the dial. A dial is drawn when its number, size or
// colors change, not on a timer.
{
const { register, el, level, limits, tokens } = Oriel;

const START = Math.PI * 0.75, SWEEP = Math.PI * 1.5; // a three-quarter ring, open at the bottom
const WORDS = { warn: "Warm", crit: "Hot" };

class Dial {
  /**
   * label, unit   under the number, and after it
   * color         token name for the arc when all is well
   * max           the value at the end of the ring
   * limits        optional () => [warn, critical], drawn as bands
   * words         what the level is called, in text as well as color
   */
  constructor(cell, { label, unit, color, max, limits, words }) {
    Object.assign(this, { cell, label, unit, color, max, limits, words });
    this.canvas = document.createElement("canvas");
    cell.appendChild(this.canvas);
    this.ctx = this.canvas.getContext("2d");
    this.value = null;
    this.note = "";
    this.key = "";
    new ResizeObserver(() => this.resize()).observe(cell);
  }
  resize() {
    const dpr = devicePixelRatio || 1;
    this.w = this.cell.clientWidth;
    this.h = this.cell.clientHeight;
    this.canvas.width = Math.round(this.w * dpr);
    this.canvas.height = Math.round(this.h * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.key = "";
    this.draw();
  }
  set(value, note = "") {
    this.value = value;
    this.note = note;
    this.draw();
  }
  max_() {
    const lim = this.limits?.();
    return lim ? Math.max(this.max, Math.ceil((lim[1] * 1.1) / 10) * 10) : this.max;
  }
  draw() {
    const { ctx, w, h, value } = this;
    if (!w || !h || document.hidden) return;
    const T = tokens;
    const lim = this.limits?.();
    const shown = value == null ? null : Math.round(value);
    const lvl = lim ? level(value, lim) : "";
    const max = this.max_();
    const key = [w, h, devicePixelRatio, shown, lvl, this.note, max, lim, T.grid, T.ink, T.muted, T.warn, T.crit, T[this.color]].join("|");
    if (key === this.key) return;
    this.key = key;

    const s = Math.min(w, h * 1.08);
    const lw = s * 0.085;
    const r = s / 2 - lw * 0.9;
    const cx = w / 2, cy = h / 2;
    const at = v => START + SWEEP * Math.max(0, Math.min(1, v / max));
    const arc = (from, to, radius, width, color, alpha = 1) => {
      ctx.beginPath();
      ctx.arc(cx, cy, radius, from, to);
      ctx.lineWidth = width;
      ctx.strokeStyle = color;
      ctx.globalAlpha = alpha;
      ctx.stroke();
      ctx.globalAlpha = 1;
    };
    ctx.clearRect(0, 0, w, h);
    ctx.lineCap = "round";

    arc(START, START + SWEEP, r, lw, T.grid);
    if (lim) {
      // the limits as thin bands inside the ring, flat-ended so they read as marks and not as values
      ctx.lineCap = "butt";
      arc(at(lim[0]), at(lim[1]), r - lw * 1.05, lw * 0.28, T.warn, 0.9);
      arc(at(lim[1]), START + SWEEP, r - lw * 1.05, lw * 0.28, T.crit, 0.9);
      ctx.lineCap = "round";
    }
    if (shown != null && shown > 0) {
      arc(START, at(Math.max(value, max * 0.012)), r, lw, lvl ? T[lvl] : T[this.color]);
    }

    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    const font = (px, weight = 400) => `${weight} ${px}px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`;
    const big = s * 0.25;
    ctx.fillStyle = shown == null ? T.muted : T.ink;
    ctx.font = font(big, 600);
    const num = shown == null ? "–" : String(shown);
    const unitPx = big * 0.42;
    // the number with its unit after it, centered as a pair
    const nw = ctx.measureText(num).width;
    ctx.font = font(unitPx, 500);
    const uw = shown == null ? 0 : ctx.measureText(this.unit).width + unitPx * 0.15;
    const x0 = cx - (nw + uw) / 2;
    ctx.textAlign = "left";
    ctx.font = font(big, 600);
    ctx.fillText(num, x0, cy + big * 0.3);
    if (uw) {
      ctx.font = font(unitPx, 500);
      ctx.fillStyle = T.muted;
      ctx.fillText(this.unit, x0 + nw + unitPx * 0.15, cy + big * 0.3);
    }
    ctx.textAlign = "center";
    const status = lvl ? this.words[lvl] : this.note;
    if (status) {
      ctx.font = font(s * 0.085, 600);
      ctx.fillStyle = lvl ? T[lvl] : T.muted;
      ctx.fillText(status, cx, cy + big * 0.3 + s * 0.14);
    }
    ctx.font = font(s * 0.1, 500);
    ctx.fillStyle = T.ink;
    ctx.globalAlpha = 0.75;
    ctx.fillText(this.label, cx, cy + r * 0.9 + lw * 0.45);
    ctx.globalAlpha = 1;

    const text = shown == null ? `${this.label}: no reading` : `${this.label}: ${shown}${this.unit}${lvl ? `, ${this.words[lvl]}` : ""}`;
    if (this.cell.getAttribute("aria-label") !== text) this.cell.setAttribute("aria-label", text);
  }
}

register("gauges", {
  name: "Gauges",
  description: "Dials for CPU and GPU load and temperature, and memory, with your warn and critical limits marked",
  color: "cpu",
  size: { w: 1.2, h: 1 },
  create(ctx) {
    const t = el(`
      <article class="tile gauges" style="--hue:var(--cpu)">
        <header><h2>Gauges</h2><span class="sub">Load and temperature</span></header>
        <div class="dials"></div>
      </article>`);
    const box = t.querySelector(".dials");
    const dials = [];
    const dial = (opts, get) => {
      const cell = el(`<div class="dial" role="img"></div>`);
      box.appendChild(cell);
      dials.push({ d: new Dial(cell, { words: { warn: "High", crit: "Very high" }, ...opts }), get });
    };
    const gpu = s => s.gpus[0];
    dial({ label: "CPU", unit: "%", color: "cpu", max: 100 }, s => [s.cpu.total]);
    dial({ label: "CPU temp", unit: "°C", color: "cpu", max: 110, limits: () => limits().cpu_temp, words: WORDS }, s => [s.cpu.temp]);
    if (ctx.info.gpus.length) {
      dial({ label: "GPU", unit: "%", color: "gpu", max: 100 }, s => (gpu(s)?.asleep ? [null, "Asleep"] : [gpu(s)?.util]));
      dial({ label: "GPU temp", unit: "°C", color: "gpu", max: 110, limits: () => limits().gpu_temp, words: WORDS }, s => (gpu(s)?.asleep ? [null, "Asleep"] : [gpu(s)?.temp]));
    }
    dial({ label: "Memory", unit: "%", color: "mem", max: 100, limits: () => limits().mem }, s => [s.mem.pct]);
    // As many columns as make the dials biggest in the room there is.
    new ResizeObserver(() => {
      const W = box.clientWidth, H = box.clientHeight, n = dials.length;
      if (!W || !H) return;
      let best = 1, size = 0;
      for (let c = 1; c <= n; c++) {
        const s = Math.min(W / c, (H / Math.ceil(n / c)) * 1.08);
        if (s > size + 1) { size = s; best = c; }
      }
      box.style.gridTemplateColumns = `repeat(${best}, minmax(0, 1fr))`;
    }).observe(box);
    return {
      root: t,
      update(s) {
        for (const { d, get } of dials) {
          const [v, note] = get(s);
          d.set(v ?? null, note);
        }
      },
    };
  },
});
}
