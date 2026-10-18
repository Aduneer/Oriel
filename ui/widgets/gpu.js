"use strict";
{
const { register, el, refs, esc, setHTML, setText, meter, fmt, facts, tempHTML, limits } = Oriel;

register("gpu", {
  name: "GPU",
  description: "One tile per graphics card: load, memory, temperature and power",
  color: "gpu",
  size: { w: 1, h: 1.2 },
  instances: info => info.gpus,
  create(ctx) {
    const g = ctx.inst;
    const t = refs(el(`
      <article class="tile" style="--hue:var(--gpu)">
        <header><h2>${g.integrated ? "iGPU" : "GPU"}</h2><span class="sub" title="${esc(g.name)}">${esc(g.name.replace(/^(NVIDIA|AMD|Intel\(R\)) /, ""))}</span>
          ${ctx.go("ms-settings:display-advancedgraphics", "Graphics settings")}</header>
        <div class="stat"><span class="big" data-k="pct">–</span><span class="facts" data-k="facts"></span></div>
        <div class="chart" data-k="chart"></div>
        <div class="meters"><div class="meter-row"><span>${g.integrated ? "Memory" : "VRAM"}</span><div class="meter" data-k="vram"><i></i></div><span data-k="vramText"></span></div></div>
      </article>`));
    const mine = s => s.gpus.find(x => x.id === g.id);
    ctx.chart(t.chart, { series: [{ label: "GPU", color: "gpu", get: s => mine(s)?.util }] });
    // An iGPU has no VRAM of its own; it borrows system memory, so count both.
    const total = g.integrated ? g.vram_total + g.shared_total : g.vram_total;
    return {
      root: t.root,
      update(s) {
        const x = mine(s);
        if (!x) return;
        t.root.classList.toggle("asleep", !!x.asleep);
        if (x.asleep) {
          setText(t.pct, "Asleep");
          setHTML(t.facts, `<span>Powered down, not polled</span>`);
        } else {
          setHTML(t.pct, fmt.pctHTML(x.util));
          setHTML(t.facts, facts([
            tempHTML(x.temp, limits().gpu_temp),
            x.power != null ? `${Math.round(x.power)}${x.power_limit ? ` / ${Math.round(x.power_limit)}` : ""} W` : "",
            x.clock ? `${(x.clock / 1000).toFixed(2)} GHz` : "",
            x.fan != null ? `Fan ${x.fan}%` : "",
          ]));
        }
        const used = g.integrated ? (x.vram_used || 0) + (x.shared_used || 0) : x.vram_used || 0;
        meter(t.vram, total ? (used / total) * 100 : 0);
        setText(t.vramText, `${fmt.gb(used)} / ${fmt.gb(total)} GB`);
      },
    };
  },
});
}
