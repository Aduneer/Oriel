"use strict";
{
const { register, el, refs, esc, setHTML, fmt, level, limits } = Oriel;

register("disk", {
  name: "Disk",
  description: "Read and write speed, how busy the disks are, and free space",
  color: "s1",
  create(ctx) {
    const t = refs(el(`
      <article class="tile" style="--hue:var(--s1)">
        <header><h2>Disk</h2>
          <span class="legend"><span><i class="key" style="--c:var(--s1)"></i> Read</span><span><i class="key" style="--c:var(--s2)"></i> Write</span></span>
          ${ctx.go("ms-settings:storagesense", "Storage settings")}</header>
        <div class="pair">
          <div><span class="label">Read</span><span class="mid" data-k="read">–</span></div>
          <div><span class="label">Write</span><span class="mid" data-k="write">–</span></div>
          <div><span class="label">Active</span><span class="mid" data-k="active">–</span></div>
        </div>
        <div class="chart" data-k="chart"></div>
        <div class="meters" data-k="drives"></div>
      </article>`));
    ctx.chart(t.chart, {
      max: "auto", floor: 1024 * 1024, fmt: fmt.rate,
      series: [
        { label: "Read", color: "s1", get: s => s.disk.read },
        { label: "Write", color: "s2", get: s => s.disk.write },
      ],
    });
    return {
      root: t.root,
      update(s) {
        setHTML(t.read, fmt.rateHTML(s.disk.read));
        setHTML(t.write, fmt.rateHTML(s.disk.write));
        setHTML(t.active, fmt.pctHTML(s.disk.active));
        if (!s.drives) return; // history samples leave drives out
        setHTML(t.drives, s.drives.slice(0, 3).map(d => {
          const pct = (d.used / d.total) * 100;
          return `<div class="meter-row"><span>${esc(d.mount)}</span><div class="meter ${level(pct, limits().drive)}"><i style="width:${pct.toFixed(1)}%"></i></div><span>${fmt.gb(d.used)} / ${fmt.gb(d.total)} GB</span></div>`;
        }).join(""));
      },
    };
  },
});
}
