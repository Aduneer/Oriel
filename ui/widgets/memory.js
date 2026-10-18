"use strict";
{
const { register, el, refs, setHTML, setText, meter, fmt, level, limits } = Oriel;

register("memory", {
  name: "Memory",
  description: "RAM in use, and committed memory against the page file limit",
  color: "mem",
  size: { w: 1, h: 1.2 },
  create(ctx) {
    const t = refs(el(`
      <article class="tile" style="--hue:var(--mem)">
        <header><h2>Memory</h2><span class="sub">${fmt.gb(ctx.info.mem_total)} GB</span>
          ${ctx.go("resource-monitor", "Open Resource Monitor")}</header>
        <div class="stat"><span class="big" data-k="used">–</span><span class="facts" data-k="facts"></span></div>
        <div class="chart" data-k="chart"></div>
        <div class="meters"><div class="meter-row" title="Memory promised to programs, backed by RAM plus the page file. Windows refuses new allocations at the limit.">
          <span>Committed</span><div class="meter" data-k="commit"><i></i></div><span data-k="commitText"></span></div></div>
      </article>`));
    ctx.chart(t.chart, { series: [{ label: "Memory", color: "mem", get: s => s.mem.pct }] });
    return {
      root: t.root,
      update(s) {
        const m = s.mem;
        setHTML(t.used, `${fmt.gb(m.used)}<small>GB</small>`);
        const lvl = level(m.pct, limits().mem);
        setHTML(t.facts, `<span>${Math.round(m.pct)}% used</span>` +
          (lvl ? `<span class="chip ${lvl}">${lvl === "crit" ? "Almost full" : "High"}</span>` : ""));
        if (m.commit_limit) {
          meter(t.commit, (m.commit_used / m.commit_limit) * 100);
          setText(t.commitText, `${fmt.gb(m.commit_used)} / ${fmt.gb(m.commit_limit)} GB`);
        }
      },
    };
  },
});
}
