"use strict";
{
const { register, el, refs, esc, setHTML, fmt, facts, tempHTML, limits } = Oriel;

register("cpu", {
  name: "CPU",
  description: "Load, clock, temperature and a bar per thread",
  color: "cpu",
  size: { w: 1, h: 1.2 },
  create(ctx) {
    const cpu = ctx.info.cpu;
    const t = refs(el(`
      <article class="tile" style="--hue:var(--cpu)">
        <header><h2>CPU</h2><span class="sub" title="${esc(cpu.name)}">${esc(cpu.name.replace(/ with .*$/, ""))} · ${cpu.cores}C/${cpu.threads}T</span>
          ${ctx.go("ms-settings:powersleep", "Power settings")}</header>
        <div class="stat"><span class="big" data-k="pct">–</span><span class="facts" data-k="facts"></span></div>
        <div class="chart" data-k="chart"></div>
        <div class="cores" data-k="cores"></div>
      </article>`));
    ctx.chart(t.chart, { series: [{ label: "CPU", color: "cpu", get: s => s.cpu.total }] });
    const bars = new Oriel.Bars(t.cores, "cpu");
    return {
      root: t.root,
      update(s) {
        const c = s.cpu;
        setHTML(t.pct, fmt.pctHTML(c.total));
        setHTML(t.facts, facts([
          c.mhz ? `${(c.mhz / 1000).toFixed(2)} GHz` : "",
          tempHTML(c.temp, limits().cpu_temp, c.temp_source),
          c.power != null ? `${Math.round(c.power)} W` : "",
          c.fans.length ? c.fans.map(f => `${f.rpm} rpm`).join(" · ") : "",
        ]));
        bars.set(c.cores);
      },
    };
  },
});
}
